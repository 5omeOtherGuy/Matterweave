# Performance campaign documents

Index of the performance campaign record. The plan, acceptance gates and measurement
conditions are in [PERFORMANCE_PLAN.md](../PERFORMANCE_PLAN.md); the runnable task
backlog and definitions of done are in [PERFORMANCE_TASKS.md](../PERFORMANCE_TASKS.md);
current state is in [STATUS.md](../STATUS.md). The showcase specification is
[SHOWCASE.md](../SHOWCASE.md).

Every other `.md` under this directory is listed below, grouped by what it is. The
campaign board is [board.json](board.json) (non-Markdown). Related evidence outside
this directory: the [first unplugged baseline](../evidence/2026-09-08-performance-baseline.md)
and the [P01 overhead check](../evidence/2026-09-08-p01-overhead.md).

## Methodology and protocols

- [measurement-v2.md](measurement-v2.md) — frame-capture schema v2: what each column means, its unit, when it is missing and what it does not measure.
- [models.md](models.md) — outcome ledger per model route: real work, observed result, acceptance limits and open gates.
- [p02-phone-runner.md](p02-phone-runner.md) — matched three-pair phone collector: variants compared, safety invariants, per-trial sequence and host verification. No phone step was executed.
- [p03/verification.md](p03/verification.md) — host-verified P03 source/mesh adapter foundation; native integration and phone checks remain open gates.
- [p03/gallery-acceptance.md](p03/gallery-acceptance.md) — acceptance procedure for the P03 native detail gallery; no visual or phone result implied.
- [p03/flora/verification.md](p03/flora/verification.md) — acceptance of the flora source data (generator 2, terrain generator 1, seed 20260908); native rendering and mobile cost are separate gates.

## Engine techniques

- [automatic-detail-engine.md](automatic-detail-engine.md) — automatic view-dependent LOD selection and derived-mesh preparation for `DetailScene`.
- [async-detail-collision.md](async-detail-collision.md) — bounded off-thread preparation of load/edit-time detail collision.
- [async-indirect.md](async-indirect.md) — bounded CPU indirect-light controller with one worker and retained copy-on-write snapshots.
- [chunk-snapshots.md](chunk-snapshots.md) — immutable shared chunk payloads via `Arc`; copy-on-write on edit.
- [ci-async-fix.md](ci-async-fix.md) — correction of the CI async-saturation test; no production scheduler bug found.
- [detail-local-loss-guard.md](detail-local-loss-guard.md) — local interior-loss guard for automatic detail selection (detail crate only).
- [detail-native-check.md](detail-native-check.md) — native check connecting `DetailScene::prepare_batches` to retained prototype geometry and instance-only updates.
- [detail-numeric.md](detail-numeric.md) — RED numeric regressions and correction for LOD numeric input validation.
- [detail-snapshots.md](detail-snapshots.md) — validation that detail volumes reuse core chunk copy-on-write.
- [frame-pacing.md](frame-pacing.md) — `matterweave-pacing`: deterministic frame pacer and honest frame-timing statistics; no efficiency, power or thermal claim.
- [indirect-light-engine.md](indirect-light-engine.md) — first diffuse indirect-light slice: CPU producer plus native integration.
- [instance-updates.md](instance-updates.md) — instance-only renderer updates using resident vertex/index buffers.
- [p03/foundation.md](p03/foundation.md) — `matterweave-detail` prototype foundation: volumes, meshing and LOD prototypes; not a shipped feature or the showcase.
- [p03/native-gallery.md](p03/native-gallery.md) — opt-in native fly/viewer mode over the P03 detail foundation; not collision or gameplay integration.
- [ray-reference.md](ray-reference.md) — CPU unit-voxel ray pack and fragment traversal reference; not a production path or measured win.
- [reflections-engine.md](reflections-engine.md) — one bounded, opt-in specular reflection bounce in the existing raster renderer.
- [renderer-comparison.md](renderer-comparison.md) — full-image comparison of raster, voxel traversal and hybrid rendering in one native example.
- [shadow-reuse.md](shadow-reuse.md) — directional shadow depth-map reuse when the depth map is unchanged.
- [stream-cancellation.md](stream-cancellation.md) — two `AsyncWorld` cancellation correctness defects fixed.
- [stream-stress.md](stream-stress.md) — sustained streaming stress example against synchronous generation and meshing.

## Slice results and showcase

- [p00.md](p00.md) — P00 preflight and lead engineering log: board, device setup, baseline hashes, cancellation rehearsal and open gates.
- [p02.md](p02.md) — dynamic-geometry reuse candidate: cache status, WSI hotspot fix, evidence and retained failure.
- [p02-analysis.md](p02-analysis.md) — executed P02 paired analysis: matched trials, statistics, thermal tradeoff and limits.
- [engine-coverage.md](engine-coverage.md) — instrumented engine source coverage over the four engine crates under llvmpipe.
- [p03/flora/README.md](p03/flora/README.md) — flora catalogue and dense test tile source data; native rendering and the full showcase remain gates.
- [showcase/muse-completion.md](showcase/muse-completion.md) — full-map showcase completion worker log (source and tests only).
- [showcase/route-fix-opus.md](showcase/route-fix-opus.md) — showcase route-fix engineering log.
- [showcase/route-replay.md](showcase/route-replay.md) — normal-runtime wetland route replay fixture and steps.
- [showcase/save-validation.md](showcase/save-validation.md) — wetland save-restoration validation.
- [reflections/README.md](reflections/README.md) — host evidence summaries for the reflection validation example.

## Execution logs

- [logs/completion-execution.md](logs/completion-execution.md) — campaign completion log: ownership/CI repair, full-map integration, native and device validation, remaining blockers.
- [logs/lead-native-p02.md](logs/lead-native-p02.md) — lead native/P02 integration judgment and accepted-revision checks.
- [logs/engine-03-collision.md](logs/engine-03-collision.md) — collision-cadence log; whole-collider AABB equivalence removed and the gate re-sourced from the owner's edit journal.
- [logs/engine-03-detail-review.md](logs/engine-03-detail-review.md) — read-only review of the engine-03 detail interior-loss guard (frozen `1a42f59`); execution not run.
- [logs/engine-03-detail.md](logs/engine-03-detail.md) — engine-03 detail log for the local interior-loss guard (detail crate only).
- [logs/engine-03-lighting-review.md](logs/engine-03-lighting-review.md) — leaf review of the async indirect/lighting delta (`577dff8`); no edits or builds.
- [logs/engine-03-ray.md](logs/engine-03-ray.md) — engine-03 ray recovery log: retained ray packing/shader work recovered after a worker timeout.
- [logs/gemini-conditions-final.md](logs/gemini-conditions-final.md) — independent correction review of the conditions validator candidate 02.
- [logs/gemini-native-cache-review.md](logs/gemini-native-cache-review.md) — independent read-only review of the native dynamic-cache/gallery change (`cc46873`).
- [logs/gemini-native-wsi-final.md](logs/gemini-native-wsi-final.md) — independent review of the final WSI/present-outcome correction (`cc46873` to `6fac9a5`).
- [logs/gemini-p00-review-a2.md](logs/gemini-p00-review-a2.md) — independent review of P00 handoff validator candidate 02.
- [logs/gemini-p00-review.md](logs/gemini-p00-review.md) — independent review of P00 handoff validator candidate 01.
- [logs/gemini-p01-final.md](logs/gemini-p01-final.md) — independent review of P01 candidate 02 and the new fixtures/validators.
- [logs/gemini-p01-schema.md](logs/gemini-p01-schema.md) — review of P01 instrumentation candidate 01: schema and column contracts.
- [logs/gemini-profile-final.md](logs/gemini-profile-final.md) — independent correction review of frozen profile validator candidate 02.
- [logs/gemini-profile-review.md](logs/gemini-profile-review.md) — independent review of the frozen profile validator slice.
- [logs/glm-p01-a1.md](logs/glm-p01-a1.md) — GLM device-condition validator slice: fail-closed two-minute idle-readiness check.
- [logs/muse-conditions-final.md](logs/muse-conditions-final.md) — read-only correction review of conditions-candidate-02 bounds and invariants.
- [logs/muse-conditions-review.md](logs/muse-conditions-review.md) — read-only review of the conditions validator and its tests.
- [logs/muse-native-cache-review.md](logs/muse-native-cache-review.md) — read-only review of the native dynamic-cache upload path.
- [logs/muse-native-wsi-final.md](logs/muse-native-wsi-final.md) — read-only review of the WSI present-outcome corrections.
- [logs/muse-p00-a2.md](logs/muse-p00-a2.md) — P00 handoff attempt-2 implementation log.
- [logs/muse-p00-review-a2.md](logs/muse-p00-review-a2.md) — P00 attempt-2 delta review of candidate 02.
- [logs/muse-p00-review.md](logs/muse-p00-review.md) — P00 read-only review of the handoff validator.
- [logs/muse-p01-behavior.md](logs/muse-p01-behavior.md) — P01 instrumentation review (behavior perspective); execution not run.
- [logs/muse-p01-final-behavior.md](logs/muse-p01-final-behavior.md) — P01 candidate-02 behavior review; no valid findings.
- [logs/muse-p01-final-lifetime.md](logs/muse-p01-final-lifetime.md) — P01 candidate-02 lifetime/concurrency review; not run.
- [logs/muse-p01-fixtures-a1.md](logs/muse-p01-fixtures-a1.md) — first deterministic authoritative-world fixture/replay slice.
- [logs/muse-p01-lifetime.md](logs/muse-p01-lifetime.md) — P01 instrumentation candidate review (concurrency/lifetime).
- [logs/muse-profile-a2.md](logs/muse-profile-a2.md) — profile validator implementation log (P01 attempt 2).
- [logs/muse-profile-a3.md](logs/muse-profile-a3.md) — profile validator correction log for the review delta.
- [logs/muse-profile-final.md](logs/muse-profile-final.md) — read-only correction review of the frozen profile validator.
- [logs/muse-profile-review.md](logs/muse-profile-review.md) — read-only review of the profile validator slice.
- [logs/opus-p01-a1.md](logs/opus-p01-a1.md) — P01 instrumentation slice 1 implementation log.
- [logs/opus-p01-a2.md](logs/opus-p01-a2.md) — P01 instrumentation correction (attempt 2) log.
- [instancing/glm-execution.md](instancing/glm-execution.md) — GLM execution log for bounded GPU prototype instancing in the render crate.
- [p03/logs/execution_log.md](p03/logs/execution_log.md) — desktop coordination log for the P03 detail/gallery lane.
- [p03/logs/opus-detail-a1.md](p03/logs/opus-detail-a1.md) — P03 detail-foundation worker log (A1).
- [p03/logs/opus-detail-a2.md](p03/logs/opus-detail-a2.md) — P03 detail-repair worker log (A2).
- [p03/logs/opus-native-gallery-a1.md](p03/logs/opus-native-gallery-a1.md) — native detail-gallery session log (A1).
- [p03/flora/logs/execution_log.md](p03/flora/logs/execution_log.md) — desktop flora-lane execution log.
- [p03/flora/logs/gemini-flora-review.md](p03/flora/logs/gemini-flora-review.md) — independent Gemini flora review of frozen candidate 02.
- [p03/flora/logs/muse-flora-delta.md](p03/flora/logs/muse-flora-delta.md) — Muse correction review of flora candidate 04.
- [p03/flora/logs/muse-flora-review.md](p03/flora/logs/muse-flora-review.md) — independent Muse flora review of frozen candidate 02.
- [p03/flora/logs/opus-finalfix.md](p03/flora/logs/opus-finalfix.md) — interrupted P03 flora final-fix note with a lead audit of premature claims.
- [p03/collision/lead-correction.md](p03/collision/lead-correction.md) — collision correction handoff (sparse masks, filtered admission).
- [p03/collision/opus-execution.md](p03/collision/opus-execution.md) — detail-scene static collision execution log; host build only, no device claim.
- [showcase-flora/muse-execution.md](showcase-flora/muse-execution.md) — Muse worker log adding three support-flora prototypes.

## Reviews

- [reviews/app-audit-astra.md](reviews/app-audit-astra.md) — independent app interaction/capture audit (frozen `9b7faa9`).
- [reviews/app-audit-astra-correction.md](reviews/app-audit-astra-correction.md) — correction review of the app interaction audit.
- [reviews/app-review-gemini.md](reviews/app-review-gemini.md) — independent Gemini app review (frozen `3f8f01d`); tests not run.
- [reviews/app-review-muse.md](reviews/app-review-muse.md) — independent Muse app review.
- [reviews/collision-review-astra.md](reviews/collision-review-astra.md) — fresh Astra collision review; no substantiated candidates.
- [reviews/collision-review-gemini.md](reviews/collision-review-gemini.md) — independent Gemini collision review; candidate findings only.
- [reviews/collision-review-muse.md](reviews/collision-review-muse.md) — independent Muse collision review; candidate findings only.
- [reviews/flora-review-gemini.md](reviews/flora-review-gemini.md) — independent Gemini flora review.
- [reviews/flora-review-muse.md](reviews/flora-review-muse.md) — independent Muse flora review.
- [reviews/map-gemini-finish.md](reviews/map-gemini-finish.md) — independent Gemini full-map findings (frozen `952d630`).
- [reviews/map-muse-finish.md](reviews/map-muse-finish.md) — independent Muse full-map findings (frozen `952d630`).
- [reviews/renderer-gemini-corrections.md](reviews/renderer-gemini-corrections.md) — Gemini review of the corrected renderer.
- [reviews/renderer-muse-corrections.md](reviews/renderer-muse-corrections.md) — Muse review of the corrected renderer.
- [reviews/replay-review-muse.md](reviews/replay-review-muse.md) — native route replay source review (frozen `c0c73e0`).
- [reviews/route-correction-gemini-a2.md](reviews/route-correction-gemini-a2.md) — Gemini route-correction review; no substantiated findings.
- [reviews/route-correction-muse-a2.md](reviews/route-correction-muse-a2.md) — Muse route-correction review; no substantiated findings.
- [reviews/route-review-astra.md](reviews/route-review-astra.md) — generator-3 route review (frozen `d58c85e`).
- [reviews/route-review-muse.md](reviews/route-review-muse.md) — generator-3 route review (frozen `d58c85e`).
- [reviews/save-review-gemini.md](reviews/save-review-gemini.md) — save-restoration review and lead disposition.

## Patch handoff contract

Workers do not commit. Each attempt has one owner, an exact base, disjoint literal
owned paths, a Pi run/session directory, an engineering log and lead-owned checks.
The lead checks ownership before dispatch. No worker may detach jobs, spawn
descendants, alter the board or access the phone.

After a worker finishes or is cancelled, the lead verifies its runner and children
are inactive, preserves tracked **and untracked** changed files, and freezes their
bytes into an artifact with a SHA-256. `verified_inactive` is a manual attestation,
not an operating-system liveness detector. Replacements increase the attempt and use
a new owner; a late old-attempt result is never accepted.

`tools/performance/check_handoff.py` is a read-only admission guard, not a board,
automatic review, authentication, process supervisor or filesystem sandbox. It checks
current run/task/attempt/owner, eligible state, inactive attestation and frozen
artifact hash. A passing submission still requires independent review and lead
acceptance. States: running, submitted, review-needed, accepted, rejected, blocked.
The JSON board and raw execution artifacts must not disagree about ownership.
