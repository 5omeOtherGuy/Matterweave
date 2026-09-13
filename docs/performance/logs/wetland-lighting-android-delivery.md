# Engineering log: wetland proxy GI/reflection Android delivery (PR #54, PR #57)

Delegated integration/acceptance worker (`w_150a72bd`) · 2026-09-13 · OnePlus 13
`192.168.178.93:5555` (CPH2653, Android 16, Adreno 830, Vulkan 1.3.284,
3168x1440). Follows the shared-settings delivery
([PR #53 log](shared-settings-android-delivery.md)); this batch owns the reviewed
proxy-GI/reflection slice and the probe hand-off.

## Actions Taken

1. Reconciled the accepted lighting slice: PR #54 code
   `28d30553606f01bdd5d38e6240782d6da552beae` + docs `3d33c4f`, corrective Opus
   review `w_adc924e0` **ACCEPT** with no blocking finding (its RED/GREEN counts
   were unverified because it was read-only; CI re-ran them here).
2. Applied the owner-directed classification fix before merge, as a wording-only
   commit `0e37e84`: the `wetland_lighting.rs` header and the repair log now say
   continuous moving-cell/body GI continuity is an **open functional
   requirement**, not Phase B cost/thermal work. No logic, budget, assertion or
   test changed; CI re-ran green on the new head.
3. Built one APK from GitHub's merge ref `e094aa5966174eff01d5edbb23b7199ab4bb0465`
   (PR #54 x `f878e1b`), tree `f52569bf9bb2bd1ed66db72f5b4e6be96501547c`, using
   the prescribed healthy environment. Predicted the final merge with
   `git merge-tree` against the post-#56 main (`e927fede…`) and confirmed the
   code delta against the built tree is **empty** (only the PR #56 docs differ),
   so the single APK is the delivery artifact.
4. Ran the narrow Android functional gate with `log.redirect-stdio true`, which
   surfaces the app's `WETLAND PROXY LIGHTING:` stderr reports in logcat as
   `I/RustStdoutStderr`. Verified stationary convergence, walk/LOD behaviour, a
   real edit rebuild and reconvergence, lifecycle, and the real dense source
   box; then an additional sustained-edit probe to force a reportable
   withdraw/reconverge cycle.
5. Restored the exact pre-test state: five owner saves byte-equal to the fresh
   backup, the settings file left absent (it was never created), `log.redirect-stdio`
   back to `false`, app force-stopped, accepted APK installed.
6. Consumed the PR #57 probe review `w_4c9f3d5e` (opus-medium, ACCEPT at
   `fae6173a` + docs `ba181f0`), applied its three non-blocking comment/log
   precision corrections as `817943d` (no logic change), retargeted the PR to
   `main` after #54 landed, and merged it as `4a64fe0` with six green checks.
   No phone gate was run for the test-only probe, per instruction.

## Delivery gates

| Criterion | Verification method | Result |
| --- | --- | --- |
| PR #54 review/CI | Corrective review at the frozen SHA, CI on the merged head | **PASS** — `w_adc924e0` ACCEPT; six checks green after the `0e37e84` wording fix |
| Android functional | Stationary convergence, walk/LOD, real edit invalidate+reconverge, lifecycle, dense box | **PASS** — see evidence below; the `MAX_MESH_PROXY_TESTS`/attach error count is zero |
| Device/source identity | APK built from GitHub's merge ref, merged tree predicted then confirmed | **PASS** — `1c857c08…` from `f52569bf`; merged `1bdfd6f`, tree `e927fede`, code delta empty |
| Save/preference integrity | Fresh 5 saves and prior settings absence before and after | **PASS** — 5/5 restored byte-exact, settings absent, `1c857c08…` installed |
| PR #57 probe | Review accepted, precision corrections, CI | **PASS** — corrections in `817943d`, six checks green, merged `4a64fe0` (no phone gate needed) |

## Device evidence (observed)

Channel: `adb shell setprop log.redirect-stdio true` made the app's stderr
reports visible as `I/RustStdoutStderr(<pid>): WETLAND PROXY LIGHTING: ...`.
Every number below is a device log line, not a host estimate.

- **Stationary convergence.** On entering the Wetland:
  `04:07:04.991 gi=false reflection=true cells=1801 pool_meshes=39 digest=Some(5591932977402440152) pending=381906 rebuild_ms=13.52`
  then `04:07:07.848 gi=true reflection=true pending=0 rebuild_ms=0.00` — GI
  converged in ~2.9 s, reflection live throughout, no error line.
- **Walk / LOD churn.** Holding the MOVE zone for 1.8 s produced no withdrawal:
  the last published state stayed `gi=true`, i.e. the digest gate absorbed camera
  and LOD churn exactly as documented, rather than a false "GI returned".
- **Real edit invalidates then reconverges.** `PLACE` logged
  `WETLAND EDIT 30.894ms Voxel added` and
  `Wetland detail geometry refreshed: source=8302 … resident_meshes=893`; the
  proxy then rebuilt (`rebuild_ms=11.14`, new digest, `cells=1801 -> 1802`) with
  `gi=false … pending=381906` at `04:09:07.400` and reconverged to
  `gi=true … pending=0` at `04:09:10.362`.
- **Lifecycle.** A real HOME (`keyevent 3`) plus `am start` resumed the app with
  functional input (holding the stick moved the diagnostics position) and no
  proxy error; reports continued with GI live.
- **Dense source box (finding 5).** The real authored clearing box
  (`BOX_HALF_EXTENT [10,5,10]` = 4000 cells / 24 000 face slots) was exercised
  with 1801-1802 occupied cells and 39 pool meshes: **zero**
  `MAX_MESH_PROXY_TESTS`, attach, pack or `Wetland proxy lighting:` errors in any
  captured window.
- **Integrity.** Pre-state: settings file absent, five saves equal to the fresh
  backup (`f0f37bcc…`, `e474b8c9…`, `44f102b1…`, `4f66fd00…`, `d53380cf…`).
  Cleanup: 5/5 restored byte-exact, settings absent, `log.redirect-stdio` false,
  app force-stopped, `1c857c08…` installed.

Artifact identity: APK
`1c857c08bdeb50678f2d634042fb9bbe32aaf406dcb889802c56462eb73fabca`
(`tools/verify_apk.py` PASS ARM64/16 KiB; v2 debug certificate; `install -r`;
on-device hash equal), merged as `1bdfd6f` with tree `e927fede…` (code paths
byte-identical to built tree `f52569bf…`). Private evidence:
`orchestration/lighting54-device/` (screenshots, per-phase logs, full logcat
windows, `prestate.json`, `cleanup.json`).

## Issues & Friction

- **The first edit produced no report.** A PLACE at `04:07:24` changed the world
  but no `WETLAND PROXY LIGHTING` line appeared in the following ten seconds,
  even though the rebuild path ran. Two honest explanations were possible —
  the edit fell outside the coverage box, or the withdraw/reconverge round trip
  fitted inside `REPORT_INTERVAL_FRAMES = 30` and returned to the previously
  reported tuple, which suppresses the report. A sustained-edit probe produced
  the full `rebuild -> gi=false -> gi=true` sequence (digest change,
  `cells=1801 -> 1802`), so the path is exercised; the report-gating gap remains
  disclosed rather than explained away.
- **Digest-gated rebuilds are invisible when the occupied cells do not change.**
  The report state tuple is `(gi, reflection, digest)`, so a rebuild that keeps
  the same digest is not a reportable state change. Device evidence of "an edit
  rebuilt the proxy" therefore requires an edit that actually changes occupied
  cells, which the probe found only on the second placement.
- **No continuous-motion observation exists.** The gate can prove convergence,
  rebuild and reconvergence, but not usable appearance while a body moves each
  frame; that needs a fixed-camera scripted path compared against per-frame
  converged references (proposed, not run).
- **The review's RED/GREEN counts were unverified in that read-only review.**
  They were re-executed by CI on the merged head instead of being taken on
  trust.

## Decisions & Rationale

- Fixed the "Phase B" classification before merge exactly as instructed, in
  comments and the repair log only: the requirement is functional (moving
  cell/body GI continuity), and labelling it cost/thermal work would have
  understated it in the durable record.
- Kept the pre-merge change wording-only so the corrective review's subject code
  remains valid apart from comments: no budget, assertion, fixture or behaviour
  was touched, and CI re-ran on the new head.
- Used `git merge-tree` to prove the post-#56 merged tree's code paths equal the
  built tree instead of rebuilding the APK: the docs PR cannot change device
  behaviour, and the equality is stronger evidence than a second binary.
- Used the app's own stderr reports as the device channel after testing that
  `log.redirect-stdio` exposes them; no source change was made to obtain
  instrumentation.
- Left the PR #57 probe un-gated on device: it is a `#[cfg(test)]` module with
  no production change, and the owner explicitly excluded a phone gate.

## Solutions Applied

- Wording-only classification fix `0e37e84`; probe precision corrections
  `817943d`; both pushed to their reviewed branches before merge.
- One APK built and installed for the #54 gate; device phases captured under
  `orchestration/lighting54-device/`.
- PR #54 merged as `1bdfd6f`; PR #57 retargeted to `main` (its original base
  branch had been merged) and merged as `4a64fe0`.

## Next checkpoint (handover)

- **PR #54 is on `main`**: merged `1bdfd6f` (tree `e927fede…`), source reviewed
  at `28d3055` + docs `3d33c4f` with the classification fix `0e37e84`; device
  APK `1c857c08…`. The next worker can treat the merged tree as device-verified
  for the paths above.
- **Unresolved dense/motion cases.** (a) Continuous moving-cell/body GI
  continuity is an open functional requirement: motion withdraws GI for the
  crossing and reconverges only after `UPDATE_BUDGET`, and no per-frame
  converged reference exists on device. (b) The report tuple can hide a fast
  withdraw/reconverge inside 30 frames. (c) The dense box was exercised for the
  real authored clearing (1802 occupied cells), not for arbitrary density up to
  `MAX_MESH_PROXY_TESTS = 4 Mi`. (d) A body-only move never retries after a
  failed attach (`build_failed` latch, disclosed by the review and unexercised
  on device). No "GI is live during motion" or full-GI/M4 claim is admissible.
- **PR #57 merged** (`4a64fe0`): the probe's own conclusion is that partial
  publication alone does not yet meet usable motion; `w_88f312a3`
  (`deepseek-indirect-retention`) is now probing indirect-dependency retention
  in `indirect.rs` + the wetland attach path, with no phone or shared-doc
  ownership.
- **PR #58 (GPU traversal, `w_17afb30e`) review is still pending final verdict**
  (`w_e3c16633` review complete, `w_538111ec` verdict running). The standalone
  cross-compiled Android executable route is feasible and will be handed over
  after this delivery; no app debug entry is needed for it.

## Unrun gates and limits

- No performance, thermal, memory or sustained-workload measurement; the
  `rebuild_ms` values are functional facts, not budgets.
- No continuous-motion appearance claim and no full-GI/M4 claim.
- Physical simultaneous multitouch and lock/unlock remain unrun device gates
  (unchanged by this batch).
- The forced attach-failure path is covered by host tests only; it was not
  injected on device.
