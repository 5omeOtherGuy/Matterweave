# muse-native-wsi-final

Actions:
- Read-only review of corrections in `/mnt/bench/matterweave-dev/worktrees/performance-p00`.
- Read `crates/matterweave-render/src/lib.rs:1007-1030` (`classify_present`), `1430-1760` (`draw` acquire/submit/present + `PresentOutcome` handling + `present_results_map_to_honest_frame_outcomes` test), `436-600` (`Swapchain::new` IDENTITY/pre-transform/FIFO, Drop idle fallback).
- Read `apps/explorer/src/gallery.rs:150-250` (request parsing/marker bound), `564-600` (`ensure_uploaded` once-per-renderer), `794-860` (capture row), lifecycle `resumed`/`suspended`/`run_exercise`.
- Read `apps/explorer/src/metrics.rs` (`FrameRow::write`, `shadow_casters_for_attempt`, `FrameLog::requested/record`), `tools/performance/check_gallery_capture.py`, `docs/performance/measurement-v2.md` overlap/fence scope.
- Did not read prior Muse/Gemini reports, did not re-audit accepted detail/flora source, no writes/shell/phone/images.

Issues/Friction — candidate defects meeting threshold: none found (0/3).
- No concrete reachable defect in `classify_present` deferral, mandatory resize/acquire-OOD/present-OOD handling, submission identity, or gallery dynamic-absent vs zero distinction.
- Deliberately not raised as findings per task: no native collision/full-map/transparent-water/instancing; `combine` budget uses `len()` payload not `capacity()` allocation (recorded limitation); absent dynamic pipeline must stay empty not fabricated zero.

Decisions/Rationale (preserved, not findings):
- `Ok(_)=>Presented{recreate:false}` is correct per Khronos SUBOPTIMAL usable-for-present; defers to explicit `resize()` or `ERROR_OUT_OF_DATE_KHR`. Avoids per-frame pipeline rebuild seen in OnePlus13 diagnostic (same extent, `Ok(true)` every present). Mandatory paths intact: `resize→recreate=true`, acquire OOD→`Retry+recreate`, present OOD→`Retry+recreate` with `submissions` already incremented (identity kept).
- Gallery row `dynamic_*=Some(0)` + `dynamic_*_ms=None→""` is direct: no dynamic pipeline exists. Differs intentionally from normal-game supported-but-skipped P02 `Some(0.0)`. Checker asserting `""` is correct; do not demand zeros.
- Retry identity handling correct: acquire-OOD `submitted=None` + masked `shadow_casters=None`; present-OOD `submitted=Some` + casters kept. `Swapchain::take` after `device_wait_idle`, per-image `finished` semaphores, `IDENTITY`/compositor rotation unchanged — no transform/shader change, lifetimes safe in single-threaded winit ordering.

Solutions: none (read-only leaf, Astra decides).

Insights:
- Acceptance check `any(mesh_sync>0)` is weak: `mesh_sync_wall_ms` wraps `ensure_uploaded()` so real upload is included, but later no-op frames also `>0` (tiny). Stronger future check could assert first captured row dominates, without changing current GREEN/RED validity. Not a defect.
- `ensure_uploaded()->Option<f64>` return is discarded (`Ok(_)=>{}`); outer `mesh_work_ms` span already captures duration, so no measurement loss. Dead param `_acquire_suboptimal` documents intentional deferral + tested combos; removal would be cleanup only.

Checks not-run (honest):
- No execution: Renderer 18 tests/Clippy, host gallery 30 frames, app 45 tests not re-run here.
- No SHA verification of `cc46873`/`6fac9a5`; no access to throwaway OnePlus13 worktree `eb045e5`, no phone comparison/lifecycle, no full native regression (lead-owned/pending per task).

