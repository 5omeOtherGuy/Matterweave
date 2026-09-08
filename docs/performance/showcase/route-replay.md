# Normal-runtime wetland route replay

## Operation

Place `wetland-replay.json` in the app files directory before entering the wetland:

```json
{"version":1,"route":"ground"}
```

The only other route value is `elevated`. The request is checked once per app
instance, after the normal menu Enter has loaded the Runtime, while focused and
outside options. It does not enter the world automatically. Unknown fields,
duplicate fields, unsupported versions, missing fields, malformed JSON and requests
larger than 4 KiB are rejected; invalid bytes remain in place. A valid request is
removed only after the initial separate result has been written successfully.
Restart the app to retry a corrected request or run another route.

The lead supplies and owns any isolated recovery fixture. This diagnostic never
constructs a journal or teleports to the route start. Initial XZ distance must be
at most 1 m from the first generated point; grounding must succeed within three
wall seconds. Ground and elevated use the actual generator vectors retained in
Runtime, not reconstructed waypoints. The generator's nominal eye height is 1.7 m.
The lead uses a separate, exclusively owned recovery fixture; other journals
remain untouched. The owner revoked repeated save backup/restore rituals. Normal
autosave and final save still save the actual current game pose to that fixture.

Movement goes through the existing `Physics::step`, fixed-step accumulator and
Vulkan draw path. Existing frame-dt clamping, collision, rendering quality and
water-depth speed selection remain unchanged: wading 2 m/s, dry walking 6 m/s.
Desired velocity is horizontal, capped near an endpoint using frame dt plus one
fixed-step allowance for the existing accumulator remainder. There are no jumps,
flight, waypoint teleports or accelerated simulation. Camera yaw follows heading;
pitch is -0.08. Arrival requires XZ distance at most 0.2 m, grounded, and eye height
within 1 m of the target plus nominal eye height.

Ground has a 600-wall-second cap, elevated 120; each waypoint has 20 seconds.
Falling more than 4 m below the target eye height or normal app respawn fails the
run. Touch/key/button activity cancels before normal input is processed; mouse
look, focus loss, suspension/HOME and exit cancel too. Options/menu activity thus
cannot silently interrupt and resume a continuous PASS. After completion the app
stays open and normal controls remain available.

## Evidence files

`wetland-replay-result-<unix-nanoseconds>-<pid>.json` is reserved exclusively and
updated via synced temporary file, rename and directory sync. It records request,
route point count, next waypoint, maximum reached index (null before any arrival),
start and actual eye, start/end Unix timestamps, monotonic wall duration,
accumulated **unclamped frame dt**, actual fixed-step count, outcome and reason.
RUNNING snapshots are logged and persisted approximately every five seconds;
PASS/FAIL/CANCEL is terminal and written once. Abrupt process kill can leave the
last RUNNING snapshot, which is not a pass. Storage errors are logged; no durability
claim can survive unavailable storage. Result files are separate from game saves.

Terminal handling saves the actual final camera/physics pose and flushes normal
capture. Profiling remains independently requested through the existing capture
mechanism and retains its ordinary physics/render rows and GPU completion joins.
Frame-dt totals and fixed-step counts are not phone FPS or mobile performance
measurements. Gather normal capture, device/driver/thermal metadata and visual
evidence separately for device acceptance.

## Engineering log

### Actions

- Added bounded replay state/request/report module and small wetland orchestration
  hooks; added only the elevated route field to Runtime's route storage.
- Added lightweight request, filesystem one-shot, off-route, settle, stall, fall,
  respawn, cancellation, completion and speed-cap regressions; no full-map rebuild
  test was added.
- Used exclusive `CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/completion-02/route-replay-target`
  and `CARGO_BUILD_JOBS=2` for Cargo verification.

### Issues

- Initial 600-second worker turn timed out. Work resumed for the authorized bounded
  finish; no worker or phone access was used.
- Initial RED: scoped test compilation failed on missing `Request`, `parse_request`
  and `Run`, the intended absent implementation. Checkpoint: `04c5f26`.
- Final formatting check initially reported layout-only differences after the last
  test addition; `cargo fmt --all` corrected them.
- Clippy emits the pre-existing vendored winit function-item integer-cast warning
  at `vendor/winit/src/platform_impl/linux/x11/ime/context.rs:161`.

### Decisions

- Reused serde/serde_json for strict schema parsing and stdlib bounded reads,
  exclusive file creation, monotonic clocks and synced atomic report replacement.
  No dependency or generator/physics/render/metrics source change.
- Rejected automatic start placement and synthetic journal construction. Fixture
  handling and native acceptance remain lead-owned.
- Kept progress timing separate from normal performance capture and final journal.

### Solutions / verification

- PASS: `cargo test --locked -p matterweave-explorer --lib wetland_replay`:
  five tests passed, 51 filtered out, 0.01 seconds on the resumed run.
- PASS: `cargo clippy --locked -p matterweave-explorer --all-targets -- -D warnings`:
  no workspace errors; upstream warning noted above.
- PASS: final `cargo fmt --all --check`, `git diff --check`, and
  `python3 tools/check_docs.py` (run before the implementation commit).
- NOT RUN: host full-app acceptance, full generated-map traversal, Android build,
  phone route/render/wading acceptance and coverage measurement. Existing host
  route results supplied by the lead are not new evidence from this leaf.

### Insights / handoff

A unit PASS proves bounded control/report behavior, not traversal feasibility or
continuous native rendering. Lead acceptance must exercise each entire route from
its legitimate start fixture, confirm no accidental Enter-release cancellation,
inspect route progress/final pose and normal capture, and deliberately interrupt
with touch and HOME. A blocked native frame cannot be preempted by this event-loop
state machine; wall timeout is evaluated when execution returns. Abrupt death
retains only the latest progress snapshot. Lead owns updates to aggregate STATUS,
roadmap, integration reviews and device evidence; this leaf changes only its
assigned source files and this log.
