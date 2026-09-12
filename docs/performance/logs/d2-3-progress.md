# D2.3 primary-plan gap: sustained-edit streaming progress (2026-09-12)

Slice: D2.3 streaming liveness, branch `engine/streaming-edit-progress`, base
`d95c63ef8268fae2f3efeb535105548d54c865e4`. Prior verdict (see
`docs/performance/logs/d2-3-streaming.md`): core rejection safety correct,
sustained-edit liveness OPEN — an edit inside every preparation interval meant no
window could ever publish without reverting an edit. This log closes that gap with
a small explicit bounded-progress policy plus the real explorer callsite.

**Verdict — sustained-edit progress implemented and host-verified; Android
acceptance NOT RUN (lead gate).** The live sandbox now advances toward the current
destination even when edits land inside every asynchronous preparation interval,
without accepting stale snapshots or losing edits. No mobile performance claimed.

## Actions Taken

1. Read `AGENTS.md`, `crates/matterweave-core/src/async_world.rs` (full),
   `crates/matterweave-core/src/streaming.rs`,
   `crates/matterweave-core/src/lib.rs` (`World::set` override semantics),
   `crates/matterweave-core/tests/async_world.rs`, the explorer tick
   (`apps/explorer/src/lib.rs` around `stream_begin`), and
   `docs/performance/logs/d2-3-streaming.md`.
2. Added a stream-specific stall counter and a bounded synchronous fallback to
   `async_world.rs` (no change to publication/rejection invariants).
3. Added two deterministic workerless regression tests in-crate: sustained-edit
   progress through the exact app-invoked sequence, and fallback
   reset/ignore controls (reversal, cancellation, success, pending work,
   mesh-only churn).
4. Proved both tests fail without the fix (mutation probe: fallback forced to
   `return false` → both fail; all 10 pre-existing lib tests still pass).
5. Wired the real explorer tick: `request_stream` → `poll_stream` → else
   `sync_fallback_if_stalled`, with `physics.sync_world` on either publication;
   camera `column_ready` gating untouched.
6. Ran scoped verification once after fixes: core tests, explorer lib tests,
   scoped Clippy, `cargo fmt --check`, `tools/check_docs.py`. Committed scoped
   code and this log; no push/PR.

## Decisions & Rationale

- **Stream-specific consecutive counter, not `discarded`.** `AsyncStats::discarded`
  merges mesh drops, saturation, cancellation and supersession, so it cannot drive
  a progress policy (explicitly excluded by the task). New
  `Queue::stale_streams`/`stale_center`, exposed as
  `AsyncStats::consecutive_stale_streams` and
  `AsyncWorld::consecutive_stale_streams()`, increments **only** when
  `poll_stream` rejects a preparation that is current in generation, seed, center
  and streaming mode but predates an edit (`source_revision != revision`).
- **Threshold 3 (`AsyncWorld::STALE_STREAM_FALLBACK_AFTER`).** Small and explicit:
  at most one synchronous rewindow per 3 consecutive edit-stale completions for
  one destination. Rationale: 1 would sync on any single bursty edit pair; much
  larger would stall visible progress for many intervals. 3 bounds sync work to
  1 window per 3 wasted preparations while advancing within 4 intervals.
- **Fallback applies to the CURRENT world/destination, never a stale snapshot.**
  `sync_fallback_if_stalled(world, eye)` runs `world.stream_around(eye)` on the
  authoritative world, so all edits survive through the stored overrides; the
  advanced revision invalidates any still-pending preparation. No stale merge by
  construction (excluded approach).
- **Per-destination counting.** `request_stream` to a new center resets the count;
  `poll_stream` rejections for a superseded center, generation/seed mismatch, or
  non-streaming mode reset instead of incrementing. Reversal therefore cannot force
  a fallback for the abandoned destination, and cancellation/world replacement
  (via `reset()`, which also clears the count) is safe.
- **No blind wall-time retries.** The policy counts stream-specific stale
  *completions*, never elapsed time or pending-queue occupancy, so ordinary
  pending work (no completion yet) never triggers a fallback.
- **Associated const, not a new export.** `lib.rs` re-exports are outside the owned
  paths, so the threshold is `AsyncWorld::STALE_STREAM_FALLBACK_AFTER` (usable
  through the already-exported type) and the counter rides on the already-exported
  `AsyncStats`. `crates/matterweave-core/src/lib.rs` untouched.
- **App wiring preserves existing safety.** Physics synchronizes through the real
  path (`physics.sync_world`) on both async publication and fallback; the
  `column_ready` camera clamp and movement gating below are unchanged.

## Solutions Applied

`crates/matterweave-core/src/async_world.rs` (only production file):

- `Queue::{stale_streams, stale_center}` + `AsyncStats::consecutive_stale_streams`.
- `AsyncWorld::STALE_STREAM_FALLBACK_AFTER: u32 = 3` with a cost-honest doc
  (one synchronous 7x7x3 window, ≤147 generated chunks, overrides preserved;
  host-measured only, no mobile claim).
- `request_stream`: destination change resets the count; destination-reached /
  non-streaming early return clears it.
- `poll_stream`: edit-stale rejection increments per-center (saturating);
  supersession/generation/seed/mode rejections reset; success resets.
- `reset()`: clears the count; `stats()` reports it.
- `consecutive_stale_streams()` accessor and
  `sync_fallback_if_stalled(world, eye)` (fires only when count ≥ threshold for
  the requested, still-outstanding destination; failed rewindow keeps the count
  for a cheap next-frame retry instead of spinning).

`apps/explorer/src/lib.rs` (11 added lines, tick only): `else if
preparation.sync_fallback_if_stalled(...)` arm with `physics.sync_world`, plus a
cost-honest comment. No other app code touched (mesh/lighting/CLI/Android
registration belong to the sibling worker).

## Issues & Friction

- **Test-design failure 1 (mine):** first draft asserted `world.get(edit)` right
  after the fallback, but the fallback had moved residency, so the origin edit was
  correctly evicted (`get == 0`, override stored). Fixed by asserting eviction
  (`get == 0`) plus override restoration on edit-free re-entry (`get == last
  material`) and full sync-replay equivalence across fallback + return.
- **Test-design failure 2 (mine):** a direct `request_stream(eye_b)` followed by
  `async_frame(eye_b)` deduped (`queued == false`, same identity, no intervening
  change). Fixed by driving reversal purely through `async_frame` (2 stales for A,
  then 1 for B → count must be 1, not 3).
- **Test-design failure 3 (mine):** leg material sequence restarted at 9 after
  `reset()`, but the world still held 9, and `set` refuses unchanged material.
  Fixed with a 7/8 leg sequence (documented inline).
- **Process nits:** an orphaned doc block after refactor (Clippy
  `empty line after doc comment`), an `assert_eq!(x, true)` (Clippy), and four
  `cargo fmt` reflows — all fixed; no production logic affected.
- **Mutation-probe hygiene:** pre-probe source copied to `/tmp` with sha256
  recorded and restored by copy (never `git checkout`), per the lesson in the
  prior log.

## Insights

- The stall was never a missing publication — it was a missing *edit-preserving
  progress* step. Counting exactly the rejection case that edits cause
  (revision mismatch with everything else current) is what makes the policy
  discriminate sustained edits from mesh churn, pending work and reversal
  without heuristics or timers.
- Per-destination counting fell out of the existing `requested_center` design:
  the controller already knew the current destination; the counter just had to be
  keyed by it.
- Async publication and the synchronous path stay bit-identical (revision,
  residency, chunk set, per-chunk revisions, all materials) even across a
  fallback + eviction + re-entry, so the fallback is observably just a
  differently-scheduled `stream_around`.

## Verification (actual results)

Target dir for every Cargo invocation:
`/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/streaming-progress-target`,
`RUSTC_WRAPPER= CARGO_BUILD_JOBS=1`. No phone, no APK, no `/mnt/bench`, no full
workspace runs.

```
$ cargo test -p matterweave-core --locked
cargo test: 76 passed (7 suites, 0.93s)      # lib 34 (was 32: +2 new), api 1,
                                             # async_world 10, replay 10, streaming 8, world 13

$ cargo test -p matterweave-explorer --locked --lib
cargo test: 157 passed, 1 ignored (1 suite, 5.96s)

$ cargo clippy -p matterweave-core -p matterweave-explorer --all-targets --locked -- -D warnings
cargo clippy: 0 errors                       # one pre-existing vendored-winit
                                             # warning (dependency, not denied)

$ cargo fmt --all -- --check                 # clean (exit 0)
$ python3 tools/check_docs.py
PASS: 210 Markdown files, 636 local links, 16 ADRs and 20 requirements.
```

Red proof (mutation probe, fallback body forced to `return false`, then restored
by copy, sha256 `a5b52f47…1148` before and after):

```
cargo test -p matterweave-core --locked --lib async_world
FAILED. 10 passed; 2 failed   # sustained_...: "exactly one bounded fallback must fire"
                              # stall_fallback_...: "leg 0: threshold reached but no fallback fired"
```

New tests (deterministic, workerless `AsyncWorld::manual()`, no sleeps):

- `sustained_edits_advance_through_the_app_invoked_fallback_without_edit_loss`:
  edit every interval toward a far window; exactly one fallback at frame
  `STALE_STREAM_FALLBACK_AFTER - 1`; eviction + re-entry preserve the last edit;
  bit-identical to the ordered synchronous replay.
- `stall_fallback_ignores_pending_and_mesh_churn_and_resets_on_reversal_cancel_and_success`:
  pending work and mesh-only churn count nothing and never fall back; reversal
  (2 for A then 1 for B → 1); success resets; `reset()` resets; two legs fire
  exactly twice (one sync per threshold); destination-reached fires nothing.

Real callsite/physics wiring: app lib tests pass (157) plus source inspection of
the tick diff (11 lines: fallback arm + `physics.sync_world`, camera gating
unchanged). No renderer/device run here.

## Remaining limits (honest)

- **Synchronous fallback cost is real:** up to one full 7x7x3 window generation
  (≤147 chunks) on the calling (main) thread per 3 consecutive edit-stale
  completions for one destination. Host-measured only — no frame-time numbers
  taken here and **no mobile performance claimed**.
- **Independent review and combined Android acceptance: NOT RUN** (lead gate).
  Device/OS/driver behaviour, thermal throttling, and physics-body support on a
  fallback-published window are unverified.
- `docs/STATUS.md`, ADRs, roadmap and native build docs intentionally untouched
  (outside owned paths / sibling worker's area); resume from this log plus the
  diff.
- Commits (branch `engine/streaming-edit-progress`, no push/PR):
  code `e2de8eae1678d225b45915aa3f37cf9853662316`, this log follows.

| Criterion | Verification | Owner | Result |
| --- | --- | --- | --- |
| Sustained-edit progress without stale snapshot acceptance or edit loss | new regression through app-used policy; fails without fix (probe) | worker | **PASS** |
| Destination/reset/cancellation and mesh churn safe | focused controls test | worker | **PASS** |
| Real sandbox callsite and physics synchronization wired | app build/tests plus source inspection | worker | **PASS** |
| Scoped checks, commit, evidence log | exact commands above; SHAs in handoff | worker | **PASS** |
| Independent review and combined Android acceptance | lead gate, never claim passed | lead | **NOT RUN** |
