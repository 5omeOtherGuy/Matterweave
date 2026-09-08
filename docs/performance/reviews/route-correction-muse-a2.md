# route-correction-muse-a2

No substantiated findings in `a22143f` scope.

- Euclidean `hypot` per-voxel corner max correctly covers diagonal corners and preserves quarter-turn invariance; empty volume still yields `0.0`.
- Segment-projection `route_clearance` with degenerate/1-point guards plus separate per-route traversal removes point-gap misses without inventing a cross-route segment; `< limit` / `>= limit` boundary matches the test.

Evidence/coverage log: Reviewed the `source_radius_m` and `route_clearance` hunks plus both call sites against the stated pre-fix voxel-corner and bracket-midpoint regressions. Coverage limited to this diff; count gates, terrain/cavity logic, and device acceptance not re-evaluated here.

