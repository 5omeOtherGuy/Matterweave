# Engineering log: delegated detail-48 device acceptance, PR #48/#51 delivery

Worker: `w_7459dd48` (delegated Android acceptance and reviewed-PR delivery; phone,
integration checkout, shared STATUS/HANDOFF/board owner for this task) · outcome:
Category: test · 2026-09-13 · run: `orchestration/detail48-acceptance/`

Continues `orchestration/deepseek-functional-delivery/result.json`. No
descendants and no extra Pi inference were used; all independent reviews cited
here were previously dispatched by Codex.

- **Scope:** accept the frozen PR #48 repair `8ae9e7d` (phase-7/retreat viewport
  fixture) on the real device and merge it if the full 14-phase + HOME/resume
  detail gate passes, then dispose of the optional CPU experiment PR #51
  (`43050cd`) against its already-completed independent corrective review. No
  performance or thermal campaign, no unrelated packages, no guard weakening.

- **Actions Taken:**
  - Reconciled the integration checkout before touching it: three untracked
    `_pending_` engineering-log stubs (`d4-2-shared-controls.md`,
    `d4-3-authoring.md`, `detail-android-convergence.md`) that blocked the
    checkout of current main were preserved privately under
    `orchestration/detail48-acceptance/reconciled-stubs/` with their SHA-256s;
    nothing tracked was overwritten. The real merged
    `detail-android-convergence.md` now comes from main.
  - Combined acceptance source in the existing acceptance checkout
    (`pr36-39-acceptance`): its branch `engine/pr36-39-acceptance` at `bbf7d1d`
    was clean and is preserved; a new local branch `engine/detail48-acceptance`
    was created from main `8183540` and `8ae9e7d` merged into it
    (`--no-ff`). Local merge head `29421d9`, tree
    `90d529a411e0b530d5933eb8d5cdde5cd008966b`.
  - One APK built with the mandated healthy environment and verified:
    `ANDROID_HOME=../android-sdk GRADLE_USER_HOME=../gradle-home
    CARGO_TARGET_DIR=../detail-android-target CARGO_BUILD_JOBS=1 RUSTC_WRAPPER=
    android/gradlew -p android :app:assembleDebug --no-daemon` (BUILD SUCCESSFUL
    in 1m03s, 5 executed/28 up-to-date). `tools/verify_apk.py` PASS (ARM64,
    16 KiB LOAD alignment), `build-tools/35.0.0/apksigner verify` PASS (v2 debug
    signer). APK
    `sha256 bff9e869aefa6448bc0b3c1d6560664f0b8b020b490cbf5283159ccf5319e01a`,
    9 088 705 bytes; installed `-r` at 2026-09-13 02:43:54 on-device hash match.
  - Detail gate run three times on the exclusive phone, always as a fresh
    one-shot: force-stop → delete stale report → write `files/engine-check.txt`
    via the mandated quoted `run-as … sh -c 'cat > files/engine-check.txt'`
    stdin path (device content verified `b'detail\n'`) → `logcat -c` → launch
    `android.app.NativeActivity`. Each run pressed a real HOME (`keyevent 3`)
    mid-run and relaunched with `am start`; **3/3 runs PASS** the full script.
  - Normal interaction gate (separate launch, no marker): chooser screenshot
    before choosing coordinates, `ENTER THE WETLAND` at 887,782, Wetland loaded
    (34 716 469 cells / 8 302 placed objects, 2.456 s), real `PLACE`/`REMOVE`
    edits, MOVE-zone walking, BREAK/GRAB/THROW dispatched, MENU and DIAGNOSTICS
    toggled, screenshots at each step.
  - Disposition of PR #51: consumed the completed independent Opus corrective
    review `w_a6d28d97` (**accept, no blockers**, all five supported findings
    resolved) and the six green checks at frozen `43050cd`, then merged the
    narrow CPU experiment as `92f0a79`. GPU, Android and cost gates are
    explicitly OPEN in the PR/body/log; issue 42 is not closed.
  - Restored all five owner saves byte-exact with the app force-stopped and left
    the accepted APK installed; verified on-device hashes and the installed
    `base.apk` hash afterwards.

- **Device gate results (OnePlus 13 CPH2653, Android 16, Adreno 830, Vulkan
  1.3.284 driver 2150760522, 3168x1440):**
  - Report (byte-identical in all three runs, sha256
    `97821a4194fc9bbd22fa14563897dc8544a8d396c2bdf2bad7b10b4feb31f300`):
    terminal `PASS detail: 14 phases …` with `self.plans.len() = 14`;
    15 `phase=` applications = phases 0-13 plus the resumed phase 3
    re-prepared; `fallback=0` on every phase line; `zero-extent=Retry;
    recreating renderer` and `phase=13 … renderer recreated, static scene
    re-installed` present.
  - Real 1440 px surface proof: phase 7 reports
    `occlusion_solid:boulder:Half` (expected `Half` at 1440/1080, `Quarter` at
    720/480) with `flora_source=6/6 flora_pixel_eligible=2`; phase 0 reports the
    deferral proof `converge_iters=2 max_prepare_builds=1
    total_converge_builds=2 geometry_changed=true`; `capabilities:` line names
    Adreno 830/Vulkan 1.3.284; screenshots are 3168x1440.
  - HOME/resume evidence: after HOME+relaunch the report contains a second
    `capabilities:`+`fixture:` pair and the in-progress phase re-applied against
    a fresh pool (`resident_prototypes` reset to 11), then continues to phase 13;
    the app process was not restarted (same pid throughout).
  - The scripted budget is 120 presented frames per phase (nominal 1 680); the
    report does not print a presented count, so no measured frame count is
    claimed.
  - Interaction: `WETLAND EDIT 11.904 ms Voxel added`,
    `WETLAND EDIT 19.234 ms Voxel removed` (app log excerpt in evidence);
    MOVE-zone walking changed the diagnostics position 85.1/19.3/53.8 →
    83.6/19.3/54.4; MENU open/close and DIAGNOSTICS overlay responsive.
    BREAK/GRAB/THROW were dispatched and stayed responsive but produced no
    object effect: diagnostics showed 6 physics bodies and 8 302 placed objects
    before and after, and no body sat under the reticle/reach. Recorded as
    dispatched misses, **not** as confirmed object effects.
  - No new ANR/crash: `dumpsys activity lastanr` reports no ANR since boot, the
    newest `/data/anr/` trace is still the 2026-09-12 23:25:55 baseline, and no
    `ANR in`/`FATAL EXCEPTION` lines appear for the app.

- **Issues & Friction:**
  - The brief's "Current APK `45ab5de`" is the historical PR #36 merge head
    quoted in STATUS, not an installed build; the installed APK at handoff was
    `0531c7d1…` (previous combined acceptance), now replaced by `bff9e869…`.
    Corrected rather than silently following the brief.
  - Harness records remain inconsistent with completed repo logs on some
    workers: `../orchestration/deepseek-phase7-repair/result.json` carries
    `"error": "engineering log unfilled"` and `../orchestration/deepseek-ray51-repair/result.json`
    the same class of error, while the in-repo logs
    (`docs/performance/logs/detail-phase7-repair.md`,
    `ray-hierarchy-review-repair.md`) are complete. Repository logs are treated
    as ground truth; the immutable harness records were not rewritten and this
    discrepancy is reported separately.
  - The detail fixture's phase-7 occlusion pair is only a few pixels tall at
    127 m depth, so the committed occlusion screenshot is nearly empty; the
    acceptance is the projected-error arithmetic and the reported LOD/overlap
    predicates, not a visible image.

- **Decisions & Rationale:**
  - Merged PR #48 only after the acceptance tree was proven equal to the actual
    GitHub merge tree (`06975ce` tree `90d529a4…` = local acceptance tree), so
    the device evidence transfers exactly to the merged commit.
  - Merged PR #51 as reviewed: the corrective review accepted the frozen head and
    all six checks passed; the merge note and PR text keep GPU/Android/cost gates
    open and issue 42 open, because no device or GPU evidence exists for the
    experiment.
  - Kept the interaction pass deliberately narrow instead of walking the world
    looking for a body to hit: the previous wave already established that
    BREAK/GRAB/THROW miss without a target in reach, and the delegated brief
    asks not to broaden interaction testing. The diagnostics body-count
    discriminator makes the miss claim falsifiable.
  - Left the acceptance worktree on `engine/detail48-acceptance` (previously
    `engine/pr36-39-acceptance` @ `bbf7d1d`, preserved) rather than deleting
    anything.

- **Solutions Applied:**
  - Marker written only through the quoted `run-as … sh -c 'cat > …'` stdin
    path with bytes `detail\n`; the one-shot marker is consumed before the
    chooser, so no stale composition can be mistaken for a fresh run.
  - Reports pulled with `run-as … cat` (read-only) and kept with SHA-256s;
    screenshots taken with `screencap -p` to an owned `/data/local/tmp` path then
    `adb pull`.
  - Saves restored file-by-file with `run-as … sh -c 'cat > files/<name>'` and
    verified by on-device `sha256sum` against the fresh backup.

- **Insights:**
  - The repaired fixture is deterministic on device: three independent runs
    (two with screenshots, all with a real HOME/resume) produced byte-identical
    reports, which is a stronger reproducibility statement than a single run.
  - The remaining non-blocking review limits are real and disclosed: the
    fixture is proven only up to a 1440 px viewport (headroom ends around
    1650-1680 px), `projected_rect` overlap is viewport-invariant so the
    four-viewport loop is redundant, and the in-run phase check verifies
    eligibility/realization/depth order but not projected overlap.

- **Verification (exact commands):**
  - `python3 tools/verify_apk.py android/app/build/outputs/apk/debug/app-debug.apk`
    → PASS; `apksigner verify --print-certs` → v2 debug certificate.
  - `adb -s 192.168.178.93:5555 install -r …app-debug.apk` → Success;
    `pm path`/`sha256sum` → `bff9e869…`.
  - Detail: `cd orchestration/detail48-acceptance && python3 run_detail_gate{,2,3}.py`
    → PASS each (`detail-report-1440-run{1,2,3}.txt`, identical SHA-256).
  - Interaction: see `device/interaction/logcat-edit-excerpt.txt`,
    `it-00…it-17*.png`, `anr-after.txt`.
  - Saves: `saves-final-verify.txt` equals the fresh backup hashes
    (`f0f37bcc…`, `e474b8c9…`, `44f102b1…`, `4f66fd00…`, `d53380cf…`).
  - `python3 tools/check_docs.py` PASS and `git diff --check` clean on the docs
    branch.

- **Not run / remaining:**
  - No performance, memory, thermal or sustained-workload measurement; the
    `WETLAND EDIT` times are functional facts, not budget claims.
  - No runtime projected-overlap proof and no viewport above 1440 px was tested;
    the fixture envelope beyond ~1650 px is unverified.
  - PR #51 has no GPU/Android/cost evidence; those gates stay OPEN and issue 42
    stays open.
  - Third-slot work (D4.2 shared mobile settings `w_6f7430c8`,
    wetland proxy lighting `w_da1d82ec`) is registered, unreviewed and
    unintegrated by this task.
