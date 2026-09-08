# CI async saturation test correction

Scope: `crates/matterweave-core/tests/async_world.rs`, base `0e5b3be`.
No production scheduler bug identified; no production changes required.

## Defect and RED

The supplied `performance/completion-02/ci-host-failure.log` lines 507–515
show all work delivered with zero discards, failing only the final discard
assertion. Queue admission refusals do not increment `discarded`; only completed
results dropped or rejected do. Eight producer passes do not guarantee the worker
has filled its result queue before `mesh_all` begins draining it. Furthermore,
`[0,20,0]` is in the absent chunk `[0,1,0]`: creating it does not guarantee that
any admitted snapshot becomes stale. Thus a correct scheduler can deliver every
latest mesh without any discard.

RED adds an explicit prerequisite: request an old snapshot of the chunk being
edited. The focused test compiles and fails at that request (line 249), proving
this fixture did not provide that chunk. This is a test-fixture regression, not
a production failure. The original focused test passed locally with CPU affinity
0; an earlier saturation-precondition probe passed 40 repetitions, so the exact
remote interleaving was not reproduced locally. Reruns are not the fix.

Commands use `CARGO_BUILD_JOBS=2` and
`CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/completion-02/ci-async-target`:

```sh
cargo test -p matterweave-core --test async_world saturated_requests_stay_bounded_and_still_deliver_the_latest_meshes -- --exact
```

RED result: 0 passed, 1 failed, 6 filtered out; failure is
`assertion failed: jobs.request_mesh(&world, [0, 1, 0])`.

GREEN verification and final changes follow in the fix checkpoint.
