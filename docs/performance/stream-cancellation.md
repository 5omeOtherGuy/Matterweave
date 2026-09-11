# Stream/mesh cancellation correctness

Scope: [`crates/matterweave-core/src/async_world.rs`](../../crates/matterweave-core/src/async_world.rs)
and its unit tests, base `a4d7ba0`, branch `engine-stream-cancel`. Two correctness
defects were fixed; bounds, world copy-on-write storage and the single-worker
scheduler are unchanged. No timing behaviour is claimed.

## What problem this solves

1. A superseded pending window could overwrite the latest request.
2. `reset` reused the cancellation token at generation exhaustion, so a stale
   in-flight result could be accepted after a reset.

## How it works

### Defect 1: superseded pending window

`request_stream` records `requested_center` (the only publishable center) and
keeps at most one pending window. When the caller returns to a center that is
already in flight while a different center is still pending, the in-flight/pending
dedupe returned early **before** dropping the stale pending window.

Sequence: request A (A pending) → worker takes A (A in flight, pending empty) →
request B (B pending) → request A again. The dedupe saw A in flight and returned,
leaving B pending. The worker then finished A (result = A), ran B last
(result = B, discarding A), and `poll_stream` rejected B because
`requested_center` was A. The caller's latest request A was computed, valid, and
still thrown away.

Fix: before deduping, drop any pending window whose identity differs from this
request and count it discarded. The latest request is the only publishable
center, so a superseded pending window can neither run last and overwrite the
wanted result nor be published for a center `poll_stream` will reject. One
pending window and one in-flight job are preserved; the replace path now only
ever fills an empty pending slot.

### Defect 2: reset at generation exhaustion

`reset` bumped the generation with `saturating_add(1)`. The generation is the
cancellation token: the worker discards any result whose stamp differs from the
current generation. At `u64::MAX`, `saturating_add` reuses the current token, so
a job in flight at that generation was **not** invalidated. If the world had not
advanced (same revision), the stale in-flight result was accepted after `reset`,
violating the reset guarantee. This also let a stale completion at the reused
token remove a live `active` key inserted by a post-reset request.

Fix: treat the generation as a never-reused opaque token. `checked_add` mints a
fresh token; at exhaustion the controller **retires** the worker
(`shutdown = true`) instead of reusing the token. `available()` then reports
false and requests are refused, so the caller keeps the authoritative synchronous
path and no stale background result can be published. Retirement at `u64::MAX`
is unreachable in practice but makes the guarantee total. Clearing `active` on
reset plus the strictly-increasing token also closes the stale-key removal.

## What was verified

Deterministic tests drive the state machine through a private `#[cfg(test)]`
manual harness (no worker thread, no public test-only API): `take_job` /
`run_job` are stepped explicitly, so ordering is fixed rather than raced.

- `latest_request_matching_inflight_drops_a_superseded_pending_window`: A in
  flight, B pending, return to A; asserts B is dropped and A publishes.
- `reset_at_generation_exhaustion_retires_instead_of_leaking_inflight_work`:
  generation set to `u64::MAX`, mesh in flight, `reset`; asserts retirement and
  that the stale in-flight mesh is discarded and new work refused.
- `reset_below_exhaustion_bumps_generation_and_keeps_running`: a normal reset
  still mints a fresh token and stays available.

Commands (target
`/mnt/bench/matterweave-dev/performance/engine-02/stream-cancel-target`,
`CARGO_BUILD_JOBS=1`):

```sh
cargo test -p matterweave-core
cargo clippy -p matterweave-core --all-targets
```

Results on this workstation (x86-64 Linux dev host; not a target-device claim):
`cargo test -p matterweave-core` — 46 passed across 7 suites. Clippy — no issues.
RED at commit `3e7b1e1` (both defect tests fail), GREEN at `d1fc54e`.

Lead integration corrections: two additional deterministic regressions exposed
buffered-A/running-B/request-A loss and delayed fallback visibility during
retirement. The first fix removed superseded pending work; the shared queue now
also tracks the requested stream identity, so a superseded completion cannot
overwrite the desired buffered result. `available()` checks retirement
immediately, even while the bounded job finishes. Lead RED `8a913e6`, GREEN
`02fc5d3`; all 48 core tests pass. Logs:
`engine-02/stream-cancel-lead-{red,green}.log`. Source sharing and the existing
queue/result caps are preserved.

## Limits and open work

- These are correctness changes, not throughput measurements.
