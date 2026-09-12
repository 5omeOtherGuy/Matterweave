# Detail diagnostic Android phase-0 convergence repair

Worker session on branch `engine/detail-android-convergence`, based on `fc6212e`
(current `main` after PR #36/#39). Scope owned by this worker:
`apps/explorer/src/detail_check.rs` and this log. No runtime, scene, physics,
wetland, fixture or renderer change.

Source of truth for the failure: `orchestration/pr36-device/detail-report-failed.txt`
(device lead's Android run; Adreno 830, Vulkan 1.3.284, driver 2150760522 —
recorded from that report, not re-run here).

- **Actions Taken:** Reproduced the Android phase-0 failure on the renderer-free
  control path, measured the distinct coarse `(prototype, Lod)` pairs each
  supported viewport selects, added the viewport regression
  `cold_lazy_convergence_defers_at_android_and_host_viewports`, then declared the
  diagnostic's own cold-phase bound
  (`COLD_PHASE_MAX_COARSE_BUILDS_PER_PREPARE = 1`, below the mirrored production
  maximum `MAX_COARSE_BUILDS_PER_PREPARE = 2`). Re-ran the focused detail gate,
  the detail crate, scoped Clippy, rustfmt and the docs checker.

- **Issues & Friction:** The device report failed with
  `FAIL detail: phase 0: bounded-convergence phase started fully resident; the
  lazy path was not exercised`, selecting `dense_control_far:Quarter`,
  `occlusion_solid:Quarter`, `solid_far:Quarter`, `solid_near:Quarter`,
  `solid_yaw_neg:Quarter`, `dense_tile_far:Source` and every flora instance at
  `Source`. Those five `Quarter` selections are only **two** distinct coarse
  pairs (`boulder Quarter`, `dense_control Quarter`) — exactly the production
  cap of two (the cap counts distinct newly built pairs per prepare) — so the
  first prepare realized both and nothing was left to defer. The pre-repair
  statement in `detail-production-fixtures.md` ("relies on at least three
  distinct coarse pairs at the declared 480 host / 1080 Android viewport
  heights") held for the host window and for an assumed 1080 px device height,
  but not for the real 1440 px surface. The pair count is viewport-sensitive:
  the same fixed 2 px error budget is harder to clear on a taller viewport
  because the same world-space error projects to more pixels, so fewer instances
  coarsen. Measured with the cold-phase camera/config (throwaway scenes):
  `1440 px -> 2` (`boulder Quarter`, `dense_control Quarter`),
  `1080 px -> 3`, `720 px -> 4`, `480 px -> 3`
  (adds `dense_tile_control Half`); an envelope probe gave 3 pairs at
  200–440 px, 3 at 1600 px and 2 at 1800–3200 px. Nothing else failed: the
  remaining 13 phases never ran because phase 0 aborts the gate.

- **Decisions & Rationale:** Alternatives considered: (a) declare a diagnostic
  cold-phase cap of 1; (b) add a third always-coarsening dense fixture prototype;
  (c) force the phase to a 480 px viewport; (d) raise the pixel budget. (c) and
  (d) manufacture the deferral instead of proving it, and (d) weakens a real
  quality guard; (b) changes prototype/instance counts and therefore the whole
  report, capability check and several unrelated tests for one phase. (a) is the
  smallest correct change and is honest: a bound tighter than production still
  exercises the production lazy path (select -> cap-limited realization ->
  authoritative `Source` fallback -> later realization), additionally proves the
  per-prepare cap is honored, and changes no quality guard, viewport, mutation or
  lifecycle semantic. The mirrored production maximum is kept as a named
  reference with a `const` assertion that the declared bound stays below it, and
  the regression pins `distinct pairs > cap` at each supported viewport so a
  future cap raise or a surface taller than the measured envelope fails loudly
  with the pair count instead of passing silently.

- **Solutions Applied:**
  - `apps/explorer/src/detail_check.rs:81` declares
    `COLD_PHASE_MAX_COARSE_BUILDS_PER_PREPARE = 1` with the viewport rationale;
    `:85` asserts it stays below `MAX_COARSE_BUILDS_PER_PREPARE` (`:64`, 2);
    `:521` is the only phase that uses it, still read through the phase's
    `LodConfig` (cap stays configurable, phase 0 stays `converge: true`).
  - `:1680`/`:1688`-`:1689` the PASS report now states the declared diagnostic
    cap and the production maximum it sits below, so device output cannot imply
    the gate ran the production cap.
  - `:2311` adds `cold_lazy_convergence_defers_at_android_and_host_viewports`
    (`:2301` viewport list `1440/1080/720/480 px`). It runs the real
    `prepare_phase` (selection, cap-limited realization, packed-instance and
    probe validation) per viewport and requires: at least two prepares, a
    first-prepare deferral, `max_builds <= cap`, a zero-deferred, zero-build,
    revision-matched stationary follow-up with non-empty resident coarse meshes.
    Liveness asserts the tile-scale control is `Source` at 1440 px and coarse at
    480 px, so the passing path is the live viewport selection, not a forced
    host size.
  - Pre-repair red (isolated run): the new test panicked
    `1440 px selects 2 distinct coarse pair(s); a cap of 2 cannot defer`.
    Post-repair green: 21/21 `detail_check` tests.
  - One-shot phase mutation and surface re-entry are untouched:
    `PhaseState`/`converged` and `suspended` semantics are unchanged and
    `phase_reentry_for_a_new_viewport_does_not_replay_one_shot_evidence` passes;
    the cap-0 `resident-zero-build-budget` stationarity phase is unchanged.

- **Insights:** The lazy-path gate was coupled to the host window size. A
  diagnostic that proves "the lazy path was exercised" must own the bound it
  needs and pin the precondition that makes it meaningful; relying on a
  production cap that a larger live viewport can satisfy in one prepare turns a
  correct engine into a failing gate. The explicit failure text (pair count,
  selected instances) is what made the distinction between a real lazy-path
  regression and a viewport-dependent fixture assumption decidable instead of
  guessed. The more robust long-term alternative, should a future device surface
  exceed the measured 2-pair envelope, is a second always-coarsening fixture
  prototype — a fixture decision, not a runtime change.

## Verification (host, debug profile)

All commands used
`RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/mesh-native-repair-target cargo …`
(the warm owned target; no release profile, no full workspace rerun).

| Check | Command | Result |
| --- | --- | --- |
| Red, pre-repair phase config | `cargo test -p matterweave-explorer --lib cold_lazy_convergence_defers_at_android_and_host_viewports -- --nocapture` | FAILED: `1440 px selects 2 distinct coarse pair(s); a cap of 2 cannot defer` |
| Green, focused detail gate | `cargo test -p matterweave-explorer --lib detail_check -- --nocapture` | 21 passed, 0 failed (includes the new viewport regression and the untouched re-entry test) |
| Related selection crate | `cargo test -p matterweave-detail --locked` | 122 passed (14 suites) |
| Scoped Clippy | `cargo clippy -p matterweave-explorer --all-targets --locked -- -D warnings` | 0 errors; only the pre-existing vendored winit `function_casts_as_integer` warning (dependency, unchanged) |
| Formatting | `rustfmt --check --edition 2021 apps/explorer/src/detail_check.rs` | clean |
| Docs | `python3 tools/check_docs.py` | PASS |

## Device gate — 2026-09-13 (delegated delivery worker, OnePlus 13)

Source tested: combined acceptance checkout `pr36-39-acceptance` at
`bbf7d1d` = `engine/interaction-delivery` `f5fc91f` + cherry-pick of `648dda2`
(full PR scopes stay separate; this merge was for one device batch only).
APK `sha256 0531c7d115fb1d3af425d87fa4404e4aefa3048dbbcef2f41ad5b890e8a88a7e`
(ARM64 16K-aligned, debug-signed, `tools/verify_apk.py` PASS, installed
`-r` 00:25:16). One-shot marker `files/engine-check.txt = detail`, fresh
Vulkan1.3.284 Adreno830 surface at 1440 px.

Result: **phase 0 now passes at the physical viewport and phases 1–6 pass; the
run stops at phase 7 (pre-existing, unrelated to this repair).**

| Step | Evidence |
| --- | --- |
| Phase 0 `cold-far-bounded-convergence` | `lods[Source=10 Half=0 Quarter=5] builds_this_call=1 geometry_changed=true coarse_realized=5 converge_iters=2 max_prepare_builds=1 total_converge_builds=2` — the deferral proof now runs at 1440 px |
| Phases 1–6 | recorded and passing (approach, retreat, mid, FOV zoom, orthographic out/in) |
| Phase 7 `occlusion-foreground-fungus-over-solid` | `FAIL detail: phase 7: occlusion foreground flora was not a pixel-eligible guard retention` |

Phase 7 is **not** caused by the cold-phase cap: the occlusion phase keeps the
default config and cap, and its in-run check requires
`coarse_would_pass_pixels(...)` for the foreground fungus. A disposable host
probe (temporary test in the acceptance checkout, reverted; not committed) over
the same fixture measured:

| viewport | projected error px | budget `2/(1+0.4)` px | eligible |
| --- | --- | --- | --- |
| 480 | 0.563 | 1.429 | yes |
| 720 | 0.844 | 1.429 | yes |
| 1080 | 1.266 | 1.429 | yes (existing pinned case) |
| 1440 | 1.688 | 1.429 | **no** |

So the occlusion fixture's guard-retention demonstration is viewport-coupled
above ~1216 px, the same class of assumption the cold phase had. It needs its
own fixture repair (viewport-robust occlusion camera/geometry) before this
diagnostic can pass at the real 1440 px surface; the delegated delivery
reported that to Codex for an isolated writer rather than expanding the fix
scope silently. HOME/resume was not reached because the run aborts at phase 7;
it must be exercised in the repaired rerun.

## Not verified / remaining gates

- **Android phase run:** PARTIAL 2026-09-13 — phase 0 fixed and verified at
  1440 px, phase 7 viewport coupling blocks the full 14-phase PASS. No phone,
  ADB, APK or `/mnt/bench` use in the authoring session.
- **Performance (NOT RUN):** no timing, memory or thermal claim; host software
  Vulkan is not device evidence.
- **Envelope bound:** the cold phase needs at least two distinct coarse pairs.
  Measured 2–4 pairs across 200–3200 px in the cold-phase camera, minimum 2
  (at 1440 px and at 1800–3200 px). A device surface outside that envelope, or a
  fixture change that removes `dense_control`/`boulder` coarse eligibility,
  fails the phase explicitly with the pair count — never silently passes.
- **`docs/STATUS.md` linkage and ADR updates:** not in this worker's owned
  scope; the lead integrates this log into the status/roadmap narrative.
