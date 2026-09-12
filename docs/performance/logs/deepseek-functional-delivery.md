# Engineering log: delegated functional-fix acceptance and delivery

Worker: `deepseek-functional-delivery` (delegated integration/acceptance owner for
this task) · outcome: Category: review · 2026-09-13 · run:
`orchestration/deepseek-functional-delivery/`

- **Scope:** independently accept and deliver exactly two finished fixes:
  interaction ANR `c1e8777` (integrated `bb68e59`, branch
  `engine/interaction-delivery`) and detail diagnostic `648dda2` (branch
  `engine/detail-android-convergence`). Android phone ownership, combined APK
  gate and reviewed/passing PR merges were delegated with this task. No
  descendants were dispatched; no thermal/performance campaign, no unrelated
  packages.

- **Actions Taken:**
  - Read AGENTS, HANDOFF, STATUS, the board, both author results and both
    completed independent reviews (`w_a6f188e8` interaction: no findings;
    `w_94385293` detail: sound + one stale fixture-log claim).
  - Verified the integrated interaction commit is byte-identical to the author's
    frozen source (`../orchestration/interaction-review-source.sha256`; all four
    files hash-match `c1e8777`) and independently re-read the changed code paths
    (`scene.rs` digest-bounds accessor, `wetland_state.rs` DDA, `wetland.rs`
    action dedup) against their baselines. Confirmed `draws()` transform equals
    `instances` transform, digest bounds formula equals
    `DetailVolume::bounds_local`, invalid-transform/empty/missing-prototype
    semantics are preserved, and Place/Remove/Grab/Break each take one ray while
    Throw takes none.
  - Combined acceptance build: fast-forwarded `pr36-39-acceptance` `45ab5de` to
    `f5fc91f`, cherry-picked `648dda2` → `bbf7d1d`; built the one batch APK with
    the mandated healthy environment (`ANDROID_HOME`, `GRADLE_USER_HOME`,
    `CARGO_TARGET_DIR=../detail-android-target`, `CARGO_BUILD_JOBS=1`,
    `RUSTC_WRAPPER=`). `tools/verify_apk.py` PASS (ARM64 16 KiB alignment),
    `apksigner verify` debug cert; APK
    `sha256 0531c7d115fb1d3af425d87fa4404e4aefa3048dbbcef2f41ad5b890e8a88a7e`.
  - Device gates on the exclusive OnePlus 13 (Vulkan 1.3.284, Adreno 830,
    3168x1440). Detail: marker `files/engine-check.txt = detail`, fresh run →
    phase 0 now passes at 1440 px, phases 1-6 pass, **phase 7 fails** on a
    pre-existing viewport-coupled fixture assumption. Interaction: fresh launch,
    chooser → Wetland, three real voxel edits (`WETLAND EDIT 13.015 ms Voxel
    added`, `16.613 ms Voxel added`, `20.232 ms Voxel removed`), all four action
    buttons plus MENU exercised, no new ANR/crash.
  - Triaged the GLM detail finding as a real stale statement and corrected it on
    the detail branch instead of leaving it unowned.
  - Restored all five owner saves byte-exact from the private backup with the app
    force-stopped; the fixed APK remains installed.

- **Issues & Friction:**
  - First action screenshots were pixel-identical to the pre-action frame even
    though the log proved `PLACE` succeeded; the app's periodic save had already
    overwritten the HUD status and the changed 13 ms voxel was not in the crop.
    The decisive evidence is the app's own `WETLAND EDIT` timing log, not frame
    diffs.
  - `BREAK`/`GRAB`/`THROW` produced no object effect because no physics object
    was inside the 6 m reach at the saved camera (27 m from the arch clearing).
    Walking toward the clearing with the touch move-zone was slow (≈0.6 m/s over
    obstructed terrain), so this run records those three as dispatched misses,
    explicitly not as confirmed object effects.
  - The detail run exposed a second 1440 px viewport coupling at phase 7 that no
    author or reviewer had reached before, because the previous device run died
    at phase 0. Host probe at the same fixture: fungus projected error 0.563 /
    0.844 / 1.266 / 1.688 px against the unchanged 1.429 px budget at
    480 / 720 / 1080 / 1440 px.

- **Decisions & Rationale:**
  - Combined APK only in the disposable acceptance checkout; the two PR branches
    keep separate commit scopes. Tested source `bbf7d1d` is the union tree of the
    two PR heads (`f5fc91f + 648dda2`), so the device result applies to both
    final scopes; only commit metadata differs.
  - Did not merge the detail PR: its DoD requires the real 14-phase + HOME/resume
    report to PASS, and phase 7 is a genuine pre-existing blocker, not a
    documentation gap. Reported the exact issue and requested an isolated
    fixture writer from Codex rather than silently expanding the fix scope.
  - Corrected the superseded fixtures-log statement on the detail branch so a
    future session cannot re-derive the wrong invariant.

- **Solutions Applied:**
  - Source/short-hash fingerprints recorded for the tested build; device evidence
    (report, logcat excerpts, screenshots, package state) kept under
    `orchestration/deepseek-functional-delivery/device/`; raw dumps and other
    apps' data never copied into Git.
  - Saves restored file-by-file through `run-as ... sh -c 'cat > files/<name>'`
    and verified by SHA-256 against `saves-before.json`.

- **Insights:**
  - Reusing the already-warmed per-revision digest removes the per-placement
    census without new cache invalidation; device edit times of 13-20 ms replace
    the recorded >5001 ms input-dispatch ANR. This is a functional timing fact,
    not a performance-budget claim.
  - Viewport-coupled fixture assumptions are the systematic failure class in the
    detail diagnostic: phase 0 (fixed here) and phase 7 (open) both assume the
    host viewport's projected pixel budget. A fixture regression that pins the
    real 1440 px surface is worth more than another camera tune.

- **Verification (exact commands, healthy environment):**
  - `python3 tools/verify_apk.py <apk>` → PASS; `apksigner verify --print-certs`
    → debug signer; APK SHA-256 as above.
  - Detail device run: report `files/detail-check-report.txt` → phase 0
    `converge_iters=2 max_prepare_builds=1 total_converge_builds=2`; phases 1-6
    recorded; phase 7 FAIL as above. Fresh report/log extracts under the device
    evidence directory.
  - Interaction device run: `logcat -s Matterweave` edits 13.015/16.613/20.232 ms;
    `dumpsys activity lastanr` unchanged at 2026-09-12 23:25:55 (baseline); no
    new `/data/anr/` traces after 23:25; no `ANR in`/`FATAL EXCEPTION` logcat
    matches; MENU open/close screen change.
  - Save integrity: all five files re-hashed on device equal to
    `saves-before.json` (`terrain-lab`, `voxel-relay`, `wetland-session`,
    `world`, `world.json.session-recovery-1`).
  - `python3 tools/check_docs.py` PASS and `git diff --check` clean on the
    branches changed for delivery.

- **Not run / remaining:**
  - Detail diagnostic 14-phase + HOME/resume device gate: NOT PASSED at 1440 px
    (phase 7 blocker); PR #48 stays unmerged until an isolated repair lands and
    the gate is rerun by this delegated owner.
  - BREAK/GRAB object-effect confirmation: not demonstrated at the saved camera
    position (reach miss), distinct from the verified dispatch/responsiveness.
  - No performance, memory or thermal measurement; none claimed.
