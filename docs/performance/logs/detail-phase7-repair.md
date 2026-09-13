# Detail diagnostic phase-7 occlusion repair at the physical viewport

Isolated fixture repair on branch `engine/detail-android-convergence` (PR #48),
based on `7466208`. Owned scope: `apps/explorer/src/detail_check.rs` and this
log. This session had no phone, no ADB, no `/mnt/bench` and no build target
other than the healthy owned `../mesh-native-repair-target`; every cargo command
ran with `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1`.

Source of truth for the failure (recorded by the delegated delivery worker, not
re-run here): `orchestration/deepseek-functional-delivery/device/detail-report-1440-fail.txt`
(OnePlus 13 CPH2653, Android 16, Adreno 830, Vulkan 1.3.284, driver
2150760522, 3168x1440 surface) and its disposable host probe
`orchestration/deepseek-functional-delivery/device/occlusion-probe.txt`:

```text
FAIL detail: phase 7: occlusion foreground flora was not a pixel-eligible guard retention
vp=480  fungus depth=92.37 eligible=true  error_px=0.563 budget_low=1.429
vp=720  fungus depth=92.37 eligible=true  error_px=0.844 budget_low=1.429
vp=1080 fungus depth=92.37 eligible=true  error_px=1.266 budget_low=1.429
vp=1440 fungus depth=92.37 eligible=false error_px=1.688 budget_low=1.429
```

- **Actions Taken:** Reproduced the device failure on the renderer-free control
  path at 1440 px (three focused tests red for the intended reason), audited
  every phase precondition at 1440/1080/720/480 px inside this same diagnostic
  instead of only the failing phase, repaired the occlusion camera and the
  retreat camera, gave the occlusion check numbers in its failure text, re-ran
  the focused and full app tests, scoped Clippy, rustfmt, the docs checker and
  the real 14-phase host Vulkan gate, then delivered two commits
  (`28b841a` red regression, `21c3ccc` fixture repair).

- **Issues & Friction:** Phase 7's own claim — the thin foreground plant is
  retained at `Source` by the geometry guards, not by distance — is a
  projected-error inequality, so it is viewport-coupled by construction: it held
  up to about 1216 px and inverted at the physical 1440 px surface. The audit
  found two more couplings of the same class that the device run would have hit
  after phase 7: the retreat camera cleared the budget for the near control's
  selected level by 0.6% at 1440 px (1.420 px of 1.429 px, so it fails on any
  surface above ~1454 px), and only three of six flora species were
  pixel-eligible at 1440 px although the phase comment and its host test claimed
  all six. The cold-phase pair count was re-checked and is unchanged (2 pairs at
  1440 px against the declared cap of 1, asserted by the existing regression).

- **Decisions & Rationale:** Alternatives considered for the occlusion phase:
  (a) move the camera to 130 m on a level axis; (b) enlarge or substitute the
  foreground species; (c) use the 0.25 m tile-scale dense solid as the rear
  occluder; (d) relax the pixel budget or the geometry guards; (e) pin the claim
  to a low viewport or to the host window only. (b) cannot help: the eligibility
  condition is `error_m * pixels_per_metre(depth) <= budget`, and a larger
  production species (0.125 m cells) needs roughly 218 m, not less; (c) changes
  the rear instance's LOD pattern, several phase reports and unrelated
  expectations for no gain once distance is fixed; (d) and (e) weaken a real
  quality guard or manufacture the precondition and were rejected outright.
  (a) is the smallest correct change: distance is the only monotone lever on a
  pixel-budget precondition that touches no prototype, guard, config or cap. The
  default 2 px budget with 0.4 hysteresis admits one 0.125 m coarse cell of the
  fungus only from about 109 m at 1440 px; 130 m leaves 14% headroom there
  (1.224 px of 1.429 px) and stays inside the budget at 1080/720/480 px, while
  the dense boulder 4 m behind it still clears its own budget and realizes
  Half/Quarter in the same frame. Making the view axis level (`eye_y ==
  target_y`) keeps the two projected boxes concentric, so depth order is purely
  along the view axis and the projected overlap holds at both the host gate
  aspect and the 3168x1440 device aspect; the swap from a 0.5/0.15 m tilt costs
  nothing and removes an accidental asymmetry between the two boxes.

  For the retreat camera the same reasoning chose 260 m over leaving 220 m: a
  precondition that passes with 0.6% headroom is a latent failure, and the
  repair keeps the phase's documented claim ("every species faces a
  pixel-eligible coarse choice") true at every supported viewport instead of
  weakening the claim. The committed regression now requires 10% headroom for
  the near control at every viewport, so a future fixture edit that re-couples
  the camera fails on the host with the measured numbers.

  The occlusion failure message now carries level, depth, error and budget for
  the live viewport, for the same reason the cold-phase message carries its pair
  count: the distinction between a real regression and a viewport-coupled
  fixture premise must be decidable from the device report alone.

  `check_frame` taking the viewport height explicitly is test enablement, not a
  behavior change: production passes `self.viewport_height()` exactly as before,
  and the in-run occlusion condition, quality thresholds, default `LodConfig`,
  mirrored production cap, cold-phase bound and the `PhaseState`/`suspended`
  one-shot lifecycle are untouched.

- **Solutions Applied:**
  - `apps/explorer/src/detail_check.rs:150` adds `OCCLUSION_EYE_M`
    (`[-20.0, 0.35, 130.0]`) and `OCCLUSION_TARGET_M` with the depth arithmetic
    above; `:613` is the only phase that uses them (name, `occlusion: true` flag
    and default config unchanged).
  - `:571` moves the retreat camera from 220 m to 260 m with its comment
    corrected to the claim the regression now enforces (`:564`).
  - `:453`/`:460` factor `coarse_projected_error_px` and
    `fresh_selection_budget_px` out of `coarse_would_pass_pixels` (`:467`), which
    keeps the identical f32 expression and comparison; `:1467` reports those
    numbers in the occlusion failure text.
  - `:1325` `check_frame` takes `viewport_height_px` (caller: `apply_phase`,
    which passes the live window height); `:1809` `projected_rect` takes an
    aspect ratio.
  - `:2415`/`:2419` add `LIVE_VIEWPORTS_PX` (1440/1080/720/480 px, the physical
    device surface first) and `VIEW_ASPECTS` (16:9 host window, 3168x1440
    device).
  - `:2195` `occlusion_phase_layers_guarded_flora_in_front_of_a_realized_coarse_solid`
    now sweeps all four viewports: fungus at `Source` with a passing coarse
    budget, rear solid coarse, non-fallback, non-empty and revision-matched,
    foreground depth ahead of the rear depth, and projected-AABB overlap at both
    aspects.
  - `:2076` `production_flora_corpus_is_held_at_source_where_pixels_would_allow_coarse`
    sweeps the same viewports: every species `Source`, every species
    pixel-eligible (was 3/6 at 1440 px), the near control coarse, and `:2108`
    its selected level clear of the budget by at least 10%.
  - `:2778` `every_phase_precondition_holds_at_every_live_viewport` drives the
    complete 14-phase script at all four viewports through the production
    control path `apply_phase_mutation -> prepare_phase -> check_frame ->
    finalize`, after the same warm-`Source` install, and additionally asserts
    the cold phase's deferral/convergence, the occlusion phase's eligible-flora
    observation and the resident phase's zero builds.

- **Insights:** A diagnostic that claims "the guard, not distance, retained
  this" is asserting an inequality about the largest surface it supports, so the
  claim has to be pinned at that surface — the host window and a 1080 px
  assumption are not evidence about the device. Preconditions that merely
  *pass* are not enough either: 0.6% headroom hid the same defect class one phase
  earlier. The cheapest robust lever for a pixel-budget premise is camera
  distance, because the projected error is monotone in depth while geometry or
  guard edits change what the phase is supposed to prove. Driving the whole
  phase script through the real check path at every supported viewport is what
  turned a single-phase defect into an audited property of the fixture.

## Viewport audit (post-repair fixture, measured by the committed tests)

Budget is the default fresh-selection threshold `2.0 / (1 + 0.4) = 1.429 px`.
Cold-phase pairs are distinct coarse `(prototype, Lod)` pairs at the cold
camera; the declared diagnostic cap is 1 against the mirrored production
maximum 2.

| Viewport | Occlusion fungus depth / coarse error px | Occlusion rear solid | Retreat near control (selected level) | Retreat flora eligible | Cold pairs / converge iterations |
| --- | --- | --- | --- | --- | --- |
| 1440 px | 127.38 m / 1.224 px | Half at 132.00 m | Quarter, 1.201 px | 6/6 | 2 / 2 |
| 1080 px | 127.38 m / 0.918 px | Half at 132.00 m | Quarter, 0.901 px | 6/6 | 3 / 3 |
| 720 px | 127.38 m / 0.612 px | Quarter at 132.00 m | Quarter, 0.601 px | 6/6 | 4 / 4 |
| 480 px | 127.38 m / 0.408 px | Quarter at 132.00 m | Quarter, 0.400 px | 6/6 | 3 / 3 |

The full 14-phase script also passes `finalize` at all four viewports (no
deferred instances, occlusion phase reports eligible flora at each viewport,
zero builds at the resident-cap phase). Fixture envelope: the occlusion claim
loses headroom above roughly 1680 px and the "every flora species is
pixel-eligible" claim above roughly 1650 px; a taller surface fails loudly with
the measured numbers rather than passing silently.

## Verification (host, debug profile)

All commands used
`RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/mesh-native-repair-target`
(the warm owned target; checked free of other users before reuse; no release
profile, no workspace rerun).

| Check | Command | Result |
| --- | --- | --- |
| Red, pre-repair fixture | `cargo test -p matterweave-explorer --lib detail_check -- --nocapture --test-threads=1` | **FAILED at 1440 px**: occlusion `1.688 px of 1.429 px at depth 92.37 m`; full-script phase 7 the same gate message; retreat `1.420 px of 1.429 px` and `3/6` flora eligible |
| Green, focused detail gate | same command | 22 passed, 0 failed |
| App tests | `cargo test -p matterweave-explorer --lib --locked` | 183 passed, 1 ignored (pre-existing) |
| Scoped Clippy | `cargo clippy -p matterweave-explorer --all-targets --locked -- -D warnings` | 0 errors; only the pre-existing vendored `winit` `function_casts_as_integer` dependency warning |
| Formatting | `rustfmt --check --edition 2021 apps/explorer/src/detail_check.rs` | clean |
| Docs | `python3 tools/check_docs.py` | PASS |
| Diff hygiene | `git diff --check` | clean |
| Real host Vulkan gate | `MATTERWEAVE_VALIDATION=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json timeout 420s xvfb-run -a cargo run --locked -p matterweave-explorer -- --detail-check --save /tmp/detail-phase7-repair/unused.json` | `PASS detail` all 14 phases at the live 480 px window (llvmpipe, Vulkan 1.4.318, validation on), phase 0 `converge_iters=3 max_prepare_builds=1`, phase 7 `occlusion_solid:boulder:Half`, no validation error or panic |

Evidence preserved outside Git in
`orchestration/deepseek-phase7-repair/`:

| Artifact | SHA-256 |
| --- | --- |
| `red-detail_check.log` | `ca7e19d06fe07ebb833c53ce3500f266b3b85a2328214adae5de71dc3ce218c4` |
| `green-detail_check.log` | `7a80ba1919e4d799b11748938435a37ba1c107f00f204cab384bee2b59dff10e` |
| `host-gate-480px-report.txt` | `6e14d4cb0313a921a1f4f7f3506f6743257b37c62ef178a5f94a77aecc38d4a3` |
| `host-gate-480px-run.log` | `4e29a9f96a190bb7f276f7b890075a91a9be652350641e48c6ea1d2f5ac204b7` |

Source fingerprints: `apps/explorer/src/detail_check.rs` is
`d138833c4a9f8f45c1cee40ae02e40334a32058eb9b943904ab2f59cb8343f3a` at the
pre-repair base `7466208` and
`813df775ee973c7ce2ce0fd0fe087f1cc3b220069283884506d6371286f10f5a` after the
repair (`21c3ccc`, tree `89eae4a076ed4fef6b9ce77c1c8fe20a236be1b6`). This log is
the only other changed file; the branch head that carries it is the frozen
review SHA reported with this delivery.

## Review brief (independent reviewer)

Review source commit `21c3ccc` (the doc-only commit on top of it carries this
log) against base `7466208`:

1. Invariants: default `LodConfig`, `max_dilation_fraction`/`max_local_loss_fraction`
   guards, mirrored production cap 2, declared cold-phase cap 1, and the
   `PhaseState`/`suspended` one-shot lifecycle must be byte-for-byte behaviorally
   unchanged; the occlusion in-run condition must be the same predicate, only
   with a richer message; no assertion may be weakened and no viewport forced.
2. Regression strength: verify that `every_phase_precondition_holds_at_every_live_viewport`
   actually drives `apply_phase_mutation`/`prepare_phase`/`check_frame`/`finalize`
   (not a re-implementation), that it fails on the pre-repair fixture at 1440 px,
   and that the occlusion regression would still catch a camera edit that
   re-couples eligibility, overlap or depth order.
3. Numbers: re-derive the eligibility arithmetic (0.125 m coarse cell, 2 px
   budget, 0.4 hysteresis, 1440 px, depth 127.38 m) and check the audit table's
   per-viewport values against the tests' printed output.
4. Scope and honesty: confirm the change is fixture-plus-diagnostics only, that
   `check_frame`'s viewport parameter is behavior-preserving, and that no device
   or performance result is claimed from host runs.

## Android 14-phase rerun (delegated device worker, exact steps)

The combined acceptance checkout must carry both PR #48 code commits plus
current `origin/main`; record its tree SHA. Do not rebuild on `/mnt/bench`, do
not inherit `sccache`, and back up the owner's saves before touching the phone.

```sh
ABS=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912
# 1. Combined acceptance source: current origin/main + PR #48 commits
#    (28b841a, 21c3ccc) in a temporary checkout; record the tree SHA.
# 2. Build + verify + install (debug, ARM64, 16 KiB alignment):
cd "$CHECKOUT" && \
ANDROID_HOME=$ABS/android-sdk GRADLE_USER_HOME=$ABS/gradle-home \
CARGO_TARGET_DIR=$ABS/detail-android-target CARGO_BUILD_JOBS=1 RUSTC_WRAPPER= \
  android/gradlew -p android :app:assembleDebug --no-daemon
python3 tools/verify_apk.py android/app/build/outputs/apk/debug/app-debug.apk
adb -s 192.168.178.93:5555 install -r android/app/build/outputs/apk/debug/app-debug.apk
# 3. One-shot detail gate: remove the stale report, write the marker, launch.
adb -s 192.168.178.93:5555 shell am force-stop dev.matterweave.explorer
adb -s 192.168.178.93:5555 shell run-as dev.matterweave.explorer rm -f files/detail-check-report.txt
adb -s 192.168.178.93:5555 shell 'run-as dev.matterweave.explorer sh -c "echo detail > files/engine-check.txt"'
adb -s 192.168.178.93:5555 logcat -c
adb -s 192.168.178.93:5555 shell am start -n dev.matterweave.explorer/android.app.NativeActivity
# 4. Require a fresh 1440 px surface, then read the report (120 frames/phase):
adb -s 192.168.178.93:5555 shell run-as dev.matterweave.explorer cat files/detail-check-report.txt
```

Acceptance requirements, all from that fresh report plus screenshots:

- a `capabilities:` line for a fresh Vulkan surface at 1440 px, and no second
  composition created by a stale run (the marker is consumed once);
- exactly 14 `phase=` lines (`phase=0` … `phase=13`), `fallback=0`, and no
  `FAIL detail:` line; terminal `PASS detail:` with `self.plans.len() = 14`;
- `phase=0 cold-far-bounded-convergence` shows the deferral proof at 1440 px
  (`converge_iters>=2`, `max_prepare_builds` = the declared diagnostic cap 1,
  `total_converge_builds>=2`, `geometry_changed=true`);
- `phase=7 occlusion-foreground-fungus-over-solid` shows `flora_source=6/6`,
  `flora_pixel_eligible>=1` and `occlusion_solid:boulder:{Half|Quarter}` — i.e.
  the guard-retained foreground over a realized coarse solid at the live
  viewport (expected `Half` at 1440/1080 px, `Quarter` at 720/480 px);
- `zero-extent=Retry; recreating renderer` and `phase=13 ... renderer recreated,
  static scene re-installed` are present, and the run includes one real
  HOME/resume with a subsequent prepare (the resume path re-enters the current
  phase; the report must show the phase re-prepared, not replayed mutations);
- capture phase screenshots 0–13 (the occlusion frame matters) and the logcat
  excerpt; report device, OS, driver, APK SHA-256, installed timestamp, the
  report SHA-256 and the exact tested tree SHA.

## Not verified / remaining gates

- **Android:** NOT RUN in this session (no phone/ADB by design). The 14-phase +
  HOME/resume gate is delegated as above; nothing here is device evidence.
- **Performance:** NOT RUN; no timing, memory or thermal claim. Host llvmpipe
  execution is functional evidence only and is never mobile performance evidence.
- **Visual quality:** the occlusion claim is per-instance LOD selection plus a
  projected-AABB overlap and depth order at ~6 px of screen height, not a pixel
  readback or a human quality judgement. Whether a few-pixel occlusion frame is
  a useful showcase remains a product question.
- **Stale documentation (out of this worker's scope):**
  [detail-production-fixtures](detail-production-fixtures.md) still describes the
  occlusion view as "the thin fungus at 92 m depth overlaps a dense boulder at
  96 m depth" (lines 114–117) and its "three distinct eligible pairs … at both
  the 480 host and 1080 Android viewport heights" paragraph predates the cold
  phase cap; the lead should fold both corrections into the integration log.
- **`STATUS.md`, HANDOFF, roadmap and the board** were not touched (lead-owned
  integration), and no ADR is affected.
