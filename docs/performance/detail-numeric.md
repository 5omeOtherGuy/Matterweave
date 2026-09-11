# LOD numeric input validation

Scope: `matterweave-detail` camera and `LodConfig` input validation. No public
interface or heuristic policy changed.

## What problem this solves

Finite input fields can still produce unrepresentable derived arithmetic. A
forward vector with components near `f32::MAX` (e.g. `1e30`) overflows an f32
squared norm to infinity, and a near-plane projection scale can overflow or
underflow f32 even when every input is finite. Either corrupts LOD selection
instead of failing.

## How it works

- Camera normalization and view-axis projection use f64 intermediates, so a
  forward vector with components `1e30` preserves the expected depth.
- Validation rejects a near-plane projection scale that overflows or underflows
  f32, including subnormal FOV and view-height cases.
- Hysteresis thresholds (`error_budget_px / (1 + hysteresis)` and
  `error_budget_px * (1 + hysteresis)`) must stay finite and positive.
- The `11795dd` refactor makes validation and selection use the same computed
  thresholds.
- A source zero error remains zero for accepted cameras.

## What was verified

Host only.

- Three numeric regressions were RED at `9ad0492`; the tests and implementation
  were integrated as `4b4496f` / `3e9f1be`, and all 103 detail tests pass.
- All 21 LOD-focused tests pass after the `11795dd` threshold refactor.

Logs: `engine-02/detail-numeric-lead-green.log` and
`detail-numeric-refactor.log` under
`/mnt/bench/matterweave-dev/performance`.

## Limits and open work

- No device speed or visual-quality advantage is inferred.
- The stopped private build target from the interrupted run was removed.
