# CI async saturation test correction

Scope: `crates/matterweave-core/tests/async_world.rs`, base `0e5b3be`. No
production scheduler bug was identified and no production code changed.

## What problem this solves

A CI host failure (`performance/completion-02/ci-host-failure.log` lines
507–515) showed all work delivered with zero discards, failing only the final
discard assertion. The defect was in the test fixture, not the scheduler.

## How it works — the corrected fixture

- Queue admission refusals do not increment `discarded`; only completed results
  dropped or rejected do. Eight producer passes do not guarantee the worker has
  filled its result queue before `mesh_all` begins draining it.
- `[0, 20, 0]` is in the absent chunk `[0, 1, 0]`: creating it does not guarantee
  that any admitted snapshot becomes stale. A correct scheduler can therefore
  deliver every latest mesh without any discard.
- The fix requests an old snapshot of the chunk being edited before the flood,
  creates the edited chunk first, and waits for the undrained flood to complete
  before asserting saturation. It observes actual queue/inflight completion with
  the existing deadline instead of relying on worker scheduling or a sleep.
- Queue and result bounds, stale revision checks and complete latest-mesh
  delivery remain asserted.

## What was verified

Host only. Commands use `CARGO_BUILD_JOBS=2` and
`CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/completion-02/ci-async-target`:

```sh
cargo test -p matterweave-core --test async_world saturated_requests_stay_bounded_and_still_deliver_the_latest_meshes -- --exact
```

RED result: 0 passed, 1 failed, 6 filtered out; the failure is
`assertion failed: jobs.request_mesh(&world, [0, 1, 0])` (line 249), proving
this fixture did not provide that chunk.

The original focused test passed locally with CPU affinity 0, and an earlier
saturation-precondition probe passed 40 repetitions, so the exact remote
interleaving was not reproduced locally. Reruns are not the fix.

Lead checks: the full core test suite and scoped Clippy pass. The focused
saturation check also passed 40 repetitions with every test thread pinned to one
available CPU. Logs: `completion-02/ci-core-green.log`, `ci-core-clippy.log`,
`ci-async-repeat.log` under the campaign artifact root.

## Limits and open work

- Remote CI must run the fix; the local reruns above are not a CI result.
