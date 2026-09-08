# Muse correction review — candidate04

Status completed; exact Contributor Free high profile; 45.67 s.

Review: frozen candidate-04 correction only. Tests NOT RUN.

Scope: `performance/desktop-02/flora-correction.patch` vs `crates/matterweave-detail/src/flora.rs`, `tests/flora.rs`, `examples/flora_gallery.rs`. Docs skipped per instruction.

Checks:
- Fractional xz: `support_height(tops,prototype,yaw,world_x,world_z)` derives `cx/cz` by floor, offsets by `world_x+rotated`; `dense_tile` passes `world_x=(x+0.5)*0.25`. New test centre/shift `0.125` vs `0.2` covers it.
- Burial: new `intersects_terrain()` iterates all source cells by centre `+0.5*fine`, yaw+translation, rejects non-AIR/WATER; called in `instance_support` and placement loop with actual candidate `Transform`.
- Connectivity: new `set_path()` x-then-z-then-y, air-only; used for fan rib/blade and rosette midrib/flank/heart. New test `components==1` for all species.
- Export honesty: gallery computes actual `distinct_types` and `thresholds_met` with per-species minimum; corridor note changed to origin-exclusion + overhang.
- Guards: `instance_support` rejects invalid `Transform`, non-0.25m tile, `min_y!=0`; docs state centre-sampled, not continuous collision.

Remaining concrete regressions: none in this narrow scope. Prior `32 intersected / 71 components` failures are directly addressed; disclosed limits — fixture-only helper, 0.25m tile, y=0 root, centre sampling, origin-only corridor — are documented, not hidden regressions.

Log: Objective: verify correction patch. Scope: patch + 3 files only. Evidence: diff hunks above, source-backed. Result: no new regression found. Uncertainty: correctness inferred from code, no execution; visual/native quality lead-owned.
