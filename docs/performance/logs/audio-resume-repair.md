# Audio adapter resume repair — 2026-09-12

Engineering log `w_adc05f59`. Worker: `deepseek-flash-go` · outcome: fixed (host-verified).
Branch `engine/sample-runtime-integration`, base (clean lead integration HEAD)
`4446af85841b4c23d0cb011542e612d4da27112f`, worktree
`/mnt/bench/matterweave-dev/worktrees/swarm-terrain-lab`.

Owned paths: `apps/explorer/src/audio_service.rs` and this log. Ownership was
expanded narrowly by the lead to `crates/matterweave-audio/src/backend/mock.rs`
for a deterministic fail-next-start test seam (additive, no production behavior
change). No Experience edits, no other audio-crate changes, no device, no push.
Only these three paths were committed; the lead's concurrent uncommitted
`docs/HANDOFF.md`, `docs/STATUS.md` and `docs/performance/board.json` edits were
left untouched.

## Actions Taken

- Recorded the base SHA and read `apps/explorer/src/audio_service.rs`,
  `crates/matterweave-audio/src/{service,error,lib,backend}.rs`,
  `crates/matterweave-audio/src/backend/mock.rs`,
  `crates/matterweave-audio/tests/recovery.rs` and the adapter's integration in
  `apps/explorer/src/experience.rs` before editing.
- Reproduced the defect as an integration bug in the adapter only: after
  `AudioService::resume` fails, the service intentionally keeps
  `health().suspended == true`, but the adapter cleared its own `suspended` flag,
  never retried resume, and treated an open-but-paused stream as usable for
  `trigger`.
- Implemented a separate, retryable foreground resume intent
  (`resume_retry_at: Option<Instant>`), a 500 ms wall-clock cooldown, and a
  running-output guard on `is_available`/`start_event`. Intentional `suspend`
  cancels the intent; a failed initial open keeps the existing
  next-lifecycle-resume policy.
- Added three focused unit tests (no sleeps): the real failed-start/recreation
  path (`fail_next_start(-899)`), the command-backpressure refusal path (kept from
  the first pass), and intentional-suspend cancellation.
- Added the one-shot `MockOutput::fail_next_start(code)` fault injection to the
  host mock after a probe proved `OutputBackend::close` is not callable from
  `apps/explorer` (trait is `pub(crate)`; see Issues & Friction).
- Ran discrimination mutations: original-like semantics make all three new tests
  fail; removing the running guard fails the open-paused test; removing the
  cancellation fails the suspend-cancels test. The mutated trees were replaced
  byte-for-byte with the verified file (`sha256 b7879dc8…`, see below).
- Ran the DoD checks on the final tree and committed only the owned paths.

## Issues & Friction

- **The suggested `OutputBackend::close` route does not compile from
  `apps/explorer`.** `OutputBackend` is `pub(crate)` in `matterweave-audio`, so
  `AudioService::mock_backend()` returns a `MockOutput` whose `close` trait method
  is unreachable downstream. Probe evidence: `error[E0599]: no method named
  'close' found for mutable reference '&mut MockOutput'` with the only suggestion
  `is_closed`. The first pass therefore used the real service backpressure refusal
  (`CommandQueueFull` during `resume`) as the discriminating failure; after the
  lead expanded ownership, the mock got the narrower `fail_next_start` seam and
  the backpressure test was retained as a second real failure path.
- The `-899` failure is not reachable through the public service API without a
  seam: mock open always succeeds and `start` only failed on a closed stream that
  could not be closed externally. The seam reproduces `StreamStartFailed` exactly,
  including the service's "drop backend, set `recovery_pending`, keep suspension"
  contract.
- A first draft used a poll-count cooldown (32 polls). The lead asked for an
  elapsed-`Instant` cooldown; the final policy is wall-clock and caps a
  permanently failing device at two reopen/start attempts per second independent
  of frame rate. Tests inject the clock (`test_now`) so no timing sleeps exist.

## Decisions & Rationale

- **Separate intent, not a second `suspended` meaning.** `suspended` stays
  "intentional app suspension" (checked first by `trigger` and `poll_device`);
  `resume_retry_at: Option<Instant>` records "foreground wants output, the service
  has not confirmed it". A single `Option` keeps intent and deadline from
  diverging.
- **Service truth decides.** `attempt_resume` records the retry deadline from
  `service.health().suspended`, not from the `Result`: `CommandQueueFull` leaves
  suspension in force exactly like a failed start, and only the service can say
  output is running.
- **Bounded cooldown.** `RESUME_RETRY_COOLDOWN = 500 ms`; a poll before the
  deadline leaves the device untouched, a poll at/after it retries. This avoids an
  open/start loop per frame while recovering promptly.
- **Drop, never queue.** `trigger` returns `Dropped(NoDevice)` whenever
  `device_running()` is false (no stream, lost stream, or still-suspended
  service). This covers both the dead-stream state and the open-but-paused
  replacement state; nothing is queued for delayed playback. `DropReason` is
  unchanged.
- **Cancellation.** `suspend` clears `resume_retry_at`; `poll_device` still
  early-returns while intentionally suspended, so a late poll cannot restart
  background audio even if intent somehow survived.
- **Failed initial open unchanged.** `attempt_resume` runs only when a device
  exists, so `poll_device` still never opens a device that was never opened; the
  next lifecycle `resume` retries, as before.
- **Mock seam additive only.** `fail_next_start` stores a `Cell<Option<i32>>`
  that `start` consumes once and reports as `StreamStartFailed`; default `None`,
  never armed in production paths, module compiled only with `backend-mock`.
- **No queue reordering.** The retry uses the service's single FIFO
  (`resume()`/`recreate_output`/`start_output`); stops queued during recovery
  remain ordered before anything queued after it, matching the existing service
  contract.

## Solutions Applied

`apps/explorer/src/audio_service.rs`

- New `device_running(&Device)` = stream properties present **and**
  `!health().suspended`; used by `is_available` and `start_event`.
- New `resume_retry_at: Option<Instant>` field plus `now()` (production
  `Instant::now()`, test override `test_now`); `RESUME_RETRY_COOLDOWN = 500 ms`.
- `resume()` clears/establishes intent via `attempt_resume()`; `suspend()` clears
  intent; `poll_device()` retries only when the deadline has passed, otherwise
  leaves the device alone.
- `attempt_resume()` centralizes the `service.resume()` call, the existing
  `CommandQueueFull`/device-failure accounting, and the service-truth deadline.
- `NoDevice`/`available`/module docs updated to describe running output and the
  retry policy.

`crates/matterweave-audio/src/backend/mock.rs`

- `fail_next_start: Cell<Option<i32>>`, `pub fn fail_next_start(&self, code)`,
  and a consume-once check at the top of `OutputBackend::start`.

## Verification

| Criterion | Result | Evidence |
| --- | --- | --- |
| Failed resume recovers without second lifecycle action (real start failure) | PASS (host) | `stolen_stream_resume_recovers_after_bounded_retry`: `-899` start refusal, failed stream closed, event dropped, one cooldown poll recreates + starts once (`stream_recreations == 1`), trigger plays. |
| Failed resume recovers without second lifecycle action (backpressure) | PASS (host) | `refused_resume_is_retried_on_cooldown_and_drops_events_meanwhile`: refused Resume command, poll before deadline inert, poll at deadline recovers. |
| Intentional suspend cancels retry; no late/stale event playback | PASS (host) | `intentional_suspend_cancels_pending_resume_retry`: intent `None` after suspend, clock advanced past deadline, 8 polls leave the service suspended and renders silent. |
| Bounded retry and existing audio semantics retained | PASS (host) | 147 explorer lib tests pass (all prior adapter + Experience lifecycle tests); 49 audio-crate tests pass; cooldown is one retry per 500 ms, not per frame. |
| Checks | PASS | Exact commands and outputs below; `check_docs.py` below. |
| Independent review, Android repeat and PR32 delivery | LEAD | Not run here: no phone, no push, no descendants. |

Exact final check outcomes (target dir
`/mnt/bench/matterweave-dev/coarse-terrain/target-lab`, `CARGO_BUILD_JOBS=2`):

```
cargo test --locked -p matterweave-explorer --lib
  test result: ok. 147 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out

cargo test --locked -p matterweave-explorer --lib -- resume
  test result: ok. 6 passed; 0 failed; 0 ignored; 142 filtered out
  stolen_stream_resume_recovers_after_bounded_retry ... ok
  refused_resume_is_retried_on_cooldown_and_drops_events_meanwhile ... ok
  intentional_suspend_cancels_pending_resume_retry ... ok
  failed_open_degrades_to_silence_and_the_next_resume_retries ... ok
  suspend_drops_events_and_resume_restores_playback ... ok
  experience::tests::no_device_before_resume_and_one_adapter_across_lifecycle ... ok

cargo test --locked -p matterweave-audio
  six suites: 21 + 21 + 2 + 4 + 1 + 0 = 49 passed; 0 failed

cargo clippy --locked -p matterweave-explorer -p matterweave-audio --all-targets -- -D warnings
  exit 0 (one pre-existing `winit` dependency warning, not in a changed crate)

cargo fmt -p matterweave-explorer -p matterweave-audio -- --check
  exit 0

python3 tools/check_docs.py
  PASS: 204 Markdown files, 622 local links, 16 ADRs and 20 requirements.
git diff --check
  clean
```

Discrimination (mutations reverted byte-for-byte; final file
`sha256 b7879dc808d44528832dfcd52e026fffbeb2122326dd51b37870cfac35cccee6`,
mock `sha256 bd4b072912769948795595392184a8dd30851308c88f77e228d5d86b592da204`):

| Mutation | Expected failure | Observed |
| --- | --- | --- |
| Original-like: no retry intent, properties-only availability/guard | all three new tests | FAILED all three |
| `suspend` does not clear the retry intent | cancellation test | FAILED `intentional_suspend_cancels_pending_resume_retry` |
| Retry kept, running guard removed from `start_event` | open-paused test | FAILED `refused_resume_is_retried_on_cooldown_and_drops_events_meanwhile` |

The mock seam itself was verified by the real-path test: `fail_next_start(-899)`
produces `last_error == StreamStartFailed { code: -899 }`, the backend is dropped
(`stream_properties() == None`), and `stream_recreations == 1` after recovery.

## Insights

- Open is not running. The service's post-failure contract ("suspension survives
  until a successful `resume`") means any adapter availability check based on
  stream properties alone is wrong for one window; the check must read
  `health().suspended`.
- One interpretation of `resume` is not enough: because the platform failure is
  asynchronous with respect to app lifecycle, the adapter must own a retry
  policy, not the service and not the gameplay pump.
- A wall-clock cooldown is preferable to a poll count here: `poll_device` cadence
  follows frame pacing, which is exactly what stalls under load; the retry cap
  should not depend on it. The test clock keeps this deterministic.
- Fault-injection seams stay honest when they reproduce real service paths: the
  mock seam only invokes the existing `start_output` failure branch, so the test
  exercises production recovery code end to end.

## Handoff

- Lead: freeze the Android build and repeat the Wetland resume on device; the
  host evidence here is not device evidence. Suggested on-device check: pause
  (HOME) and resume with a stolen stream, then confirm recovery to `Started`
  events and progressing `callbacks`/`frames` without a second app lifecycle
  resume.
- The `fail_next_start` seam is host-only (`backend-mock`); it does not change any
  production path or the Android backend.
- Uncommitted lead-owned docs (`docs/HANDOFF.md`, `docs/STATUS.md`,
  `docs/performance/board.json`) were present in the worktree during this task and
  were not touched or committed by this worker.
