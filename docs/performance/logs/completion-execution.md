# Campaign completion execution log

## Ownership and CI repair
- **Actions Taken:** Read current requirements and verified clean f679c40, draft PR8, reachable wireless phone, no active workers/test supervisor, no collision implementation. Took sole board/phone/integration ownership. Added missing Ubuntu X11 keyboard library to CI.
- **Issues & Friction:** Previous board called unstarted collision work running. Host CI panicked loading libxkbcommon-x11.so; Android/docs passed. Full showcase still absent.
- **Decisions & Rationale:** Continue existing branch and code. Retire prior lead per owner; no repeated save backup/restore ritual. Keep legacy saves isolated through normal app mode paths. Preserve all density and acceptance targets.
- **Solutions Applied:** Board now identifies sole owner and truthful pending collision state. CI dependency repair will be verified by actual workflow.
- **Insights:** Working flora viewer is reusable input, not full-map completion.

## Source assets, reviews and first full map
- **Actions Taken:** Pushed c6e2317; host, Android and docs CI passed. Opus delivered fine-source collision with 42 physics tests reported passing. Muse delivered three support flora sources; lead reran eight focused tests and scoped Clippy after corrections. Added lead instance-isolated edits and attached bracket source; focused tests passed. Began separate-save normal wetland UI integration.
- **Issues & Friction:** First map worker hit its 1,200 s deadline with substantial source/manifest but unfinished tests/log. Collision prechecks conflated decorative occupancy with solid costs; 4M aggregate limit cannot hold the actual 8.8M unique-cell map. GLM needed a long read/reasoning phase before producing renderer changes.
- **Decisions & Rationale:** Continued the exact idle map session for a bounded finish; no duplicate generator. Independent Muse/Gemini reviews precede lead acceptance. Correct collision admission accounting rather than dropping source walls. Keep new UI unaccepted until compiled and exercised.
- **Solutions Applied:** Flora branch taper, lily-notch test and exporter class metadata corrected from review evidence. Requested collision budget/filter corrections and map test handoff.
- **Insights:** Initial map manifest reports 32,930,597 expanded occupied cells, 5,367 flora instances and 3,968,891 flora cells; only six archetypes so far. These are generated-source counts, not native delivery or performance acceptance. Phone is charging; no matched measurement is running.

## Correction and integration wave
- **Actions Taken:** Integrated flora and fine-source collision. Lead verified 44 physics tests and scoped Clippy; fresh Pi Astra review returned no substantiated candidates. Rendered and inspected bracket front/side/underside/silhouette with readable shelf attachment, bands and pores. Wired normal world chooser, separate wetland save journal, custom touch zones and typed opt-in capture (not compiled/accepted yet).
- **Issues & Friction:** Opus correction exited Interrupted; free Muse follow-up hit 429 before edits. Second map run timed out while final suite/manifest commands ran; an orphaned manifest process survived runner termination. Map tests caught actual cavity/footing disagreement at cell centres; focused repairs passed but full suite remained unverified.
- **Decisions & Rationale:** Lead completed missing collision tests instead of retrying a rate-limited writer. Frozen map submitted for independent Muse/Gemini review; new lead-owned full suite writes a durable log. Keep worker timeouts distinct from acceptance.
- **Solutions Applied:** Stopped the identified orphaned manifest child before new tests. Collision scratch uses sparse 4096-bit masks, 16M aggregate filtered-source admission and unchanged 262144 box cap; tests cover decorative-count rejection errors and partition clearance.
- **Insights:** Runner deadlines alone did not terminate every shell descendant; inspect processes after timeout. Phone observed 52%, charging, 32C; cooling requested, no benchmark started. New source counts do not establish successful traversal or mobile performance.

## Frozen map and graphics verification
- **Actions Taken:** Lead reran original full-map 14-test release suite: all passed in 218.98s. Muse/Gemini independent reviews identified unprotected elevated route, missing open-route endpoint and phantom cavity surface queries. Dispatched scoped Muse correction/catalog integration on isolated map branch. Integrated frozen map/GPU APIs for app compilation.
- **Issues & Friction:** GLM second attempt timed out after source/test/example compilation but no native run. Lead native Vulkan run emitted INDEX_BUFFER-usage validation errors despite the smoke example printing success. Independent Muse review found the same source defect. Gemini renderer review timed out without handoff.
- **Decisions & Rationale:** Native validation output overrides smoke success text and worker status. Keep frozen reviewer worktree unchanged; apply fixes on integration branch. Preserve original six flora while adding four to the actual map and protecting both routes.
- **Solutions Applied:** Corrected static index-buffer usage and added instancing native smoke plus validation-error gate to hostCI. Added opt-in wetland procedural water highlights/ripple normals/material variation for native visual review; legacy rendering remains its comparison reference.
- **Insights:** Source tests alone cannot validate Vulkan flags. Map's passing suite missed feature coverage found by independent reviews. No Android full-map claims yet.

## Continuation: real full-map execution and correctness fixes
- **Actions Taken:** Re-read the supplied attachment and live source/process/PR/device state. No prior worker remained active. PR8 host/Android/docs already passed at c6e2317. Continued bounded Opus map correction and independent Pi reviews. Reproduced blocked full map entrance, corrupt-save lockout, native unfocused smoke stall, orphan menu GPU completion, old-generator journal reuse, diagonal source-radius underestimate and between-waypoint plant intrusion. Added actual runtime/native/source regressions and repaired each verified issue.
- **Issues & Friction:** Initial Xvfb default hardware ICD lacked DRI3; stopped that owned process and selected installed lavapipe. Native 25 frames passed but strict capture correctly rejected orphan completion 105, absent from recorded submissions. Map Opus reached 900s; a test descendant outlived the runner, then was verified terminal before freezing 952d630. Its full suite was 16/17, not accepted. Muse/Gemini map review first attempts timed out; exact-session no-tool finalization returned independent findings. Tool-free app correction review attempts also timed out; do not count those as successful reviews.
- **Decisions & Rationale:** Entrance uses actual capsule/finest-source colliders and a bounded 1m lift at the same position. Recovery retains invalid/old-generator bytes. Only recorded submissions may join GPU completions. Terrain cavity assertions exclude separately placed flora: point[97.375, 16.125, 58.125] intersected parasol cap material 21, with no solid terrain. Source radius includes every occupied voxel corner; route checks use continuous segments and never connect separate routes. No density target reduced. Generation advances to 2 so old instance edit IDs cannot mutate changed placements.
- **Solutions Applied:** Commits 57131c6/e0d9188, b4e7830/625da73, 532f5d6/41e8af2, 949da07/8008271, 437b846/a22143f retain reproductions and verified fixes. Native verifier passes 25 frames/20 valid rows/six bodies/separate save/no Vulkan validation errors. Physical phone development APK SHA256 fffb3844c1e9eff6ff48f5232d3c4831b2b90553319904ef81215423f0b777de passes ordinary chooser, full map entry, bank movement and Home/resume with a separate wetland save. App now stopped. Raw logs/APK/captures/manifests under `/mnt/bench/matterweave-dev/performance/completion-02`.
- **Insights:** Source/unit checks did not prove loadability, continuous clearance or valid capture joins. Phone initial scene has 32,369,772 expanded cells and 6716 total placements; logged 1.306s CPU preparation is not first-frame latency or matched performance. That working tree APK precedes generator 2 and final map acceptance; no release provenance or optimization win claimed. Current Python discovery is 130 total tests (31 handoff+65 conditions+34 profile), correcting earlier handoff double-counting as 226.

### Final generator 2 host verification

At `7e1143e`, full native capture passes 25 frames/20 valid typed rows with six
bodies and no Vulkan validation errors (`completion-02/capture-final.log`). The
workspace suite passed 238 tests/one ignored; that expensive runtime test was then
explicitly run on the final map and passed in 16.45s (`runtime-final-map.log`).
Strict workspace Clippy passes; an upstream vendored winit warning remains.
Manifest now identifies host/debug assertions honestly rather than claiming a
release build. It reports 34,864,520 expanded occupied cells, 8,876,712 unique cells,
6,221 plants/ten species and 45,328,920 source mesh bytes; generator 2, seed20260908,
composition `f458591e7b345546`. Files: `completion-02/fullmap-final/`.

The aggregate mesh-byte gate previously rescanned all cell counts for every
prototype. `ea23de0` introduces the missing cache-byte getter test; `9a9acd4`
returns the already maintained cache counter. Identical budget test passes in 2.31s.
This is a host tooling fix, not a claimed phone rendering advantage.

A bounded Muse worker now owns only a full-source continuous Rapier traversal
regression and its log in `completion-traversal`, base `7e1143e`, run directory
`completion-02/traversal-muse`. Lead retains phone ownership and integration.

### Device validation and newly reproduced traversal blockers

Full ten-species Android build/verify/install succeeds at 7e1143e, APK 35cbd2e6…;
see [device evidence](../../evidence/2026-09-08-full-wetland-development.md).
Source remove/place persists across relaunch, generator 1 retains original bytes,
legacy world hash unchanged. HOME/resume rendering observed; captured epochs are
only 1, so no claim of renderer destruction/recreation. Both 1800/180-row captures
validate. Final release and complete quality/performance/traversal gates remain open.

Muse traversal attempt hit provider 429 and incorrectly repeated the already-fixed
raw entrance check. Lead stopped its exact runner, confirmed all children exited,
retained partial source and wrote a smaller actual-app-equivalent regression.
`3af4115`: both routes RED. Rapier's default disables autostep. A30cm step improves
progress but ground point 12/elevated point 8 still stall; 55cm provides no further gain.
`4575e79`: isolated quarter-stair RED. `9b7faa9`: bounded 30cm stair GREEN, 45 physics
tests plus Clippy pass; existing walls/ceilings/body behavior checks remain intact.
Opus route correction owns only generator/tests/log in completion-route-fix;
initial dependency includes provisional 55cm physics and must be checked with the
accepted 30cm setting during integration. No density target may be lowered.

Unused completion-map target cleaned with cargo clean (71.5 MiB), after no processes
used it. Other active/shared targets retained. PR8 title/body updated via REST
because installed gh pr edit hit the classic-Projects GraphQL deprecation. All
host/Android/docs checks pass at 7a30e5b. PR stays draft while actual route gates fail.

### Independent app audit corrections

Pi Astra medium read-only review of 9b7faa9 found internal DDA-corner placement
could be diagonal and main-menu keyboard actions could edit the hidden retained
runtime. Both reproduced under tests at cecd895. dfd7bcc advances tied ray axes
without inventing grazing hits but selects one exposed face for placement; solid
side faces are refused. Main-menu gameplay actions return before changing runtime.
App 50 normal tests and the explicit full-map menu/edit/collision/reload test pass;
workspace Clippy passes. Review log: reviews/app-audit-astra.md; bounded correction
review resumes its exact session. No GPU joining defect was identified.

Ordinary phone walking also entered the deep basin; saved eye 10.767m below the 12m
surface and underwater tint observed. No teleport/flying used. Full designated
routes and destruction remain pending. App remains force-stopped after checks.

## Generator3 integration and independent review

`cce16dd` preserves Opus's failed route experiment; `0d8225b` applies the lead
correction. See route-fix-opus.md for rejected anchors/duration trials. Final
immutable authored profile removes per-column locks; solid cave roofs supply real
walking support; flood-filled edges prevent snapping isolated terraces. Density
thresholds unchanged. Source manifest generation completed with composition
`dfb9f40519a3c151`,34,716,467 expanded cells, 6,192 plants/10 species, 47,255,040 mesh bytes.
A manifest command accidentally used the route worktree target from integration;
final manifest regenerated from the exclusive lead target, same content hash.

Actual checks: workspace 240 PASS; full Runtime 16.85s PASS; detail 19 PASS and new
waterside gate PASS; both continuous integration routes PASS 197.46667/11.35simsec,
waterside prefix 7.1166673simsec. Native/phone timing are distinct. Muse and Pi Astra
source reviews complete; Astra watersidegap corrected 60abc39/f887d67. First
waterside endpoint predicate missed the quantized anchor and produced 492m; gate
rejected it. Correct westshore predicate yields 21 points with actualwater access.

Android 0d8225b APK 4e47b6d98a2a7ab1a71ba70387334cf3571a8b06b95524d75a4fbca04e378e70
built 61s, signature/16KiB checks PASS, installed OnePlus 13. Normal chooser/entry,
movement/jump and generator 3 recovery2 save observed. Device artifacts completion-02/
phone-generator3. Phone force-stopped afterward. Not merged/released; no full phone
route or optimization result claimed.

P02 gen2 candidate 6e50f5fe/reference 4736de6f APKs frozen but NOT MEASURED. The new
collector 749bd56 was NOT EXECUTED: lead caught cleanup deleting unowned 127/profile
on preflight failure, missing base/slot 128 selection gaps, finite/scene validation
gaps. Exact Opus session resumed for correction and fault-injection tests.
A separate Pi Astra leaf owns opt-in actual app route diagnostics; phone owner remains
lead. No old user-save backup/restore ritual is used.

Generator 3 native Vulkan/Xvfb/lavapipe capture PASS: 25 frames/20 valid typed rows,
six bodies, separate save and no validation errors. Artifactcompletion-02/
wetland-capture-generator3. No mobile performance inference.

## Native replay and active fresh comparison

Astra route replay c0c73e0 integrated 4e899b3 after bounded 600+180s handoff.
55 app tests and independent Muse review pass. Native actual-app regression 73c9b87
failed because headless Xvfb has no focus; 2b08194 uses the same explicit bounded
smoke allowance as rendering. Corrected run advances and records terminal CANCEL;
25 frames/20 valid rows/six bodies/no Vulkan errors. APK 4e899b3 built 57s and passes
signature/alignment; copied to completion-02/phone-replay, not installed while
paired collection owns the phone.

Generator 3 phone destruction fixture used exclusively owned recovery 127 (initial
absence verified), normal Enter and touch BREAK. Six bodies became 29; visible
pieces settled and the saved snapshot contains 29. Grab/throw gestures had no
confirmed outcome, so not accepted. 900 profile rows validate but the capture ended
before the fracture action; do not label these destruction timing. The app was
force-stopped and only the owned recovery 127 removed. Ordinary saves untouched.

P02 collector corrections integrated aab1346: ownership-aware cleanup, missing-base
and higher-slot rejection, exact scene counts, empty edit fixture, real frame
validator. Lead also removed a fabricated+30s readiness timestamp from the unused
worker revision; actual monotonic sixth observation is now checked and regression
covered. All 160 Python tests pass. A1 rejected randomized APK path `~`; f6c4177
corrects its allowlist. A2 rejected a nonexistent 127 because adb exec-out returned
0 with a cat error on stdout; actual ls/test-f proved absence.0f8fbdc uses shell-v2
-T remote status. Neither failed run collected frames or deleted unowned files.

A3 trial 1 fresh reference completed at 09:32:02 UTC: 120s warmup/120s measurement,
480 compositor dumps and 3354 typed app rows. Exact 34864520 cells/8324 instances loaded.
Reference readiness battery 32.2–32.6C, skin 31.751–32.216C, 120.19996s span.
Later matching starts are actively collected with unchanged 1C/2C limits. Run-level
pair analysis remains open; no speedup or energy claim. GLM owns bounded offline
analysis tooling, not phone data collection.

Opus save validation stopped on explicit five-hour allowance 429/reset 1788866400.
Partial code attempted cell undo after rejected journals; lead found COW prototype
identity/allocation was not restored. Muse now owns true candidate isolation and
actual Runtime regressions. No partial recovery code is integrated. Completed
route-fix/CI/reference private targets cleaned (316/88/588 MiB); shared lead target
and caches retained. Replay worker cleaned its private target after verification.


### Save restoration and first paired evidence

Worker Muse also exhausted free quota; Astra finished source/tests, timed out
while documenting, then resumed 100s and committed f50f114 before its final handoff
was interrupted. Its actual log/test artifacts were inspected. Lead integrated
1bc2be4, reproduced missing pose rejection (dae066f RED), and completed candidate
collision/pose/body/mesh validation 221f720.62 app tests including the full Runtime
recovery pass 17.43s; actual static-collider budget regression passes 0.06s; strict
workspace Clippy passes 11.40s. Gemini has a bounded read-only candidate review.
The worker's exclusive save-validation target was cleaned after its tests stopped.

GLM's 300s analysis attempt produced no code. Lead's offline tool 4bf700e has 10
focused checks and 170 total performance Python tests pass. A3's first pair matches
metadata and visible entrance content; mean 73.551/24.260ms, median 66.325/16.583ms,
p95116.065/33.166ms, no unsupported compositor gaps, process core equivalent
0.599/0.465. These are one pair, generator 2, two-minute measurements after warmup;
not a repeated/sustained/energy/GEN3 claim. Thermal status changes remain reported.

A3 trial 3 failed before launch after 31 cooling observations. Cleanup removed only
owned recovery 127 and stopped the app. A4 began 09:56:39 UTC with remaining 2 pairs
and 61-observation bounded cooling allowance; temperature/readiness limits stay
unchanged. An old cap test initially expected 32 to fail; corrected to reject 62
and accept 61, then all 170 Python tests passed. A4 already running during that
boundary-test correction; no capture controls changed. A4 process handle 66060
owns the phone. Combined completed ordering is planned AB/AB/BA across separate
raw batches; no timestamps/files are spliced or relabeled.
