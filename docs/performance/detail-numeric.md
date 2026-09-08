# LOD numeric input validation

GLM5.3 Flash/high added three RED numeric regressions at `9ad0492` on its branch,
then timed out with a useful partial correction. Lead integrated the tests and
implementation as `4b4496f` / `3e9f1be`; all103 detail tests pass.

Finite input fields can still produce unrepresentable derived arithmetic. Camera
normalization and view-axis projection now use f64 intermediates; a forward
vector with components1e30 preserves the expected depth. Validation rejects a
near-plane projection scale that overflows or underflows f32, including subnormal
FOV/view-height cases. Source zero error remains zero for accepted cameras.
Hysteresis thresholds must remain finite and positive; lead's `11795dd` refactor
uses the same computed thresholds for validation and selection. All21 LOD-focused
tests pass after that refactor. No public interface or heuristic policy changed.

Logs: `engine-02/detail-numeric-lead-green.log` and `detail-numeric-refactor.log`
under `/mnt/bench/matterweave-dev/performance`. No device speed or visual-quality
advantage is inferred. The worker's stopped private build target was removed.
