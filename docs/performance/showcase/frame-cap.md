# Wetland frame-rate target experiment

## Implementation log — 2026-09-08

Bounded leaf based on `ce89834`; implementation `549e045`. Only wetland runtime,
journal, their tests and this log are owned here. Lead owns integration, STATUS /
roadmap updates, native/Android verification and all phone work.

The owner's two actual same-quality generator2 P02 pairs report candidate mean
presentation interval 24.26 ms versus reference 73.55/75.82 ms, with lower candidate
CPU/memory but candidate skin 49.2/49.9°C and thermal status 2 versus reference
39.1/39.5°C and status 0. These supplied measurements describe a throughput/thermal
tradeoff, **not an efficiency win**. This change enables a separate upcoming cap
experiment; this leaf collected no device measurements.

- `SavedWetland.frame_rate` accepts only 30 or 60 through existing load/save
  validation. Missing fields default to 60 using serde; journal version stays 1.
  Invalid journals retain the existing separate-recovery-file behavior.
- Runtime restores and saves the target. Normal options show `FRAME RATE: 60`
  or `FRAME RATE: 30`; touching the fifth row toggles the target and marks dirty.
  Six rows use the existing rectangles at `80 + 55*i`; Return to Menu is sixth.
- Scheduling uses stdlib integer `div_ceil` and `Duration`, preserving the
  existing 16,667 µs default and 66,667 µs menu interval; 30 Hz uses 33,334 µs.
  Deadlines remain relative to the current frame start, without catch-up bursts.
  A target is a ceiling, not promised actual FPS.
- Real elapsed dt, its existing 0.1-second clamp and fixed-step physics are
  unchanged. No resolution, shadows, flora, source density, route, collision,
  renderer, capture schema or version changes. Existing focus/background guards,
  event wake behavior and pre-click replay cancellation remain unchanged.

## Executed verification

All Cargo commands used:

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/completion-02/frame-cap-target
export CARGO_BUILD_JOBS=2
```

Commands ran synchronously with a 600-second tool timeout; no detached builds.
After verification, no build/test process remained and `lsof +D` found no open
files in the exclusive target; that generated target was removed. Shared caches
and other targets were not removed.

| Command | Actual result |
| --- | --- |
| `cargo test --locked -p matterweave-explorer frame_rate_journal -- --nocapture` before implementation | Runtime RED: 1 failed; old-journal assertion returned `Null`, expected `60`. Checkpoint `f086b39`. |
| `cargo test --locked -p matterweave-explorer full_wetland_load_edit_collision_and_reload -- --ignored` before implementation | Compile-time RED: missing Runtime `frame_rate` and app `frame_interval`, 9 errors. Checkpoint `fdeb3af`. |
| `cargo test --locked -p matterweave-explorer` after implementation | PASS: 64 passed, 1 opt-in full-map test ignored. |
| `cargo test --locked -p matterweave-explorer full_wetland_load_edit_collision_and_reload -- --ignored` after implementation | PASS: 1 passed, 17.10 s host test execution. |
| `cargo clippy --locked -p matterweave-explorer --all-targets -- -D warnings` | PASS; unchanged vendored winit function-to-integer-cast dependency warning. |
| `cargo fmt --all -- --check` | PASS. |
| `git diff --check` | PASS; source diff inspected against `ce89834`. |
| `python3 tools/check_docs.py` | PASS: 132 Markdown files, 278 local links, 15 ADRs, 20 requirements. |

Journal regression exercises real JSON/file serialization, default60, explicit30,
rejection of 0/15/59/61/120/u32::MAX, refused invalid saves and byte-preserving
recovery selection. Extended full-map regression opens ordinary options, touches
the actual cap rectangle, verifies both toggle directions/dirty state/intervals,
then persists30 through real Runtime save/recovery/reload alongside existing
source-edit, collision, jump, body and invalid-candidate restoration assertions.

No coverage percentage, native presentation timing, Android build, physical touch,
phone routes, energy or thermal benefit is established by these host checks.
Lead must integrate, verify native/app behavior and phone controls/routes/replay
cancellation/lifecycle, then run a matched-quality 30/60 cap comparison with actual
presentation and thermal evidence. Preserve device/build/scene/seed and starting
thermal conditions; do not relabel earlier generator2 trials as capped results.
