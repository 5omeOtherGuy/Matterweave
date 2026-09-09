# Current status

Updated: 2026-09-08

## Active engine completion campaign

The full engine objective remains open. Owner steering on 2026-09-08 reaffirms
that the engine is the product. Advance rendering, physics, streaming, lighting
and measured efficiency under [PERFORMANCE_PLAN](PERFORMANCE_PLAN.md) and
[PERFORMANCE_TASKS](PERFORMANCE_TASKS.md). The [showcase](SHOWCASE.md) is a test
workload; further showcase polish and old demo-save compatibility must not delay
remaining M2–M6 engine requirements. The sole current integration checkout is
`/mnt/bench/matterweave-dev/worktrees/performance-p00`, branch `codex/engine-completion-03`.
The [board](performance/board.json) records ownership; the prior two-lead arrangement
is retired. This continuation owns phone, integration and delivery.

## Current M6 audio-service trial (eval/glm-audio)

Branch `eval/glm-audio` (base `5b90975`) adds the reusable bounded audio
service: crate `matterweave-audio` with a deterministic mixer core, a game-facing
handle API (register/play/stop/gain/suspend/resume, 32 clips, 8 voices, 4 MiB
PCM, 64 commands, explicit rejection instead of stealing/overwriting) and a real
Android AAudio backend through the pinned `ndk 0.9.0` bindings plus `ringbuf
0.5.1`. Decision record: [ADR-0016](adr/0016-audio-service.md) (Proposed).

Verification actually executed for this trial:

| Check | Result |
| --- | --- |
| Baseline at frozen base | PASS: `cargo test --workspace --locked` exit 0 before changes. |
| Crate behavioral tests | PASS: 22 tests (4 suites): two-voice fixture within 1e-6, limits at/over and after reuse, atomic rejection, stale handles, saturation/backpressure recovery, suspend/resume continuation policy, fault-injected device loss with recreation, threaded mixer interleaving. |
| Real-time allocation detector | PASS: zero allocations and deallocations across 10,000 mixer callback invocations including completion and stop handling (dedicated test binary). |
| Formatting / Clippy | PASS: `cargo fmt --check`; `cargo clippy -p matterweave-audio --all-targets -- -D warnings`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; Android-target scoped Clippy for the `backend-android` feature. |
| Workspace tests / docs | PASS: `cargo test --workspace --locked` exit 0 after changes; `python3 tools/check_docs.py` PASS. |
| Android cross-compilation | PASS: diagnostic example builds for `aarch64-linux-android` (debug and release) with the pinned NDK 28.2.13676358 API-28 linker, links `libaaudio`; release artifact SHA-256 `5b2a66036e6b94d1dacecaea013bef4344df603de29a0b23b75db3efce221c2b`. |
| Host diagnostic | PASS: negotiated properties, nonzero frames, suspend/resume continuation, controlled recreation, ten open/play/stop/close cycles (mock backend; silent by design). |
| Reserved-device diagnostic | NOT RUN: no Android device was attached during the trial window (`adb devices` empty). Executable, checksum and exact procedure are delivered; final acceptance requires the coordinating reviewer to execute it. |

Limitations: no resampling and no compressed formats (48 kHz f32 mono/stereo
only; a device that cannot negotiate that fails open explicitly); a failed
stream close aborts through the ndk wrapper's drop contract; the on-device
recreation check is a controlled close/reopen, while true device-loss behavior
is validated through shared-atomics fault injection on host and the AAudio
error-callback wiring compiled on device. The earlier `engine-audio-service`
worktree (Hy4 trial) was preserved untouched and not copied.

## Current engine advancement — shadow reuse

The reusable Vulkan renderer now reuses unchanged directional shadow depth and
invalidates it for geometry, light-matrix and resource changes. Implemented at
`78cba4a`; no sample gameplay or save behavior changed in this engine slice.
257 workspace tests pass (3 existing ignored gates), strict Clippy passes, and
both30-frame native Vulkan cache/app checks pass without validation errors.
The ARM64 APK `e4f83255…` built and ran on OnePlus13 Android16:37-point continuous
physics route passed;730 valid profile rows include284 completed shadow-map reuses
and445 depth updates. No measured heat/energy improvement is claimed.
See [implementation and evidence](performance/shadow-reuse.md).

The P02 collector finished all3 matched pairs across A3/A4 and exited cleanly.
The phone is idle after the engine functional check; no collector or worker owns it.
PR8 merged at `ba6c894` after all required checks passed;
[v0.4.0 prerelease](https://github.com/5omeOtherGuy/Matterweave/releases/tag/v0.4.0)
is published with APK, manifests and both evidence archives. Next engine capabilities are automatic
detail selection and indirect illumination/reflections, plus remaining streaming,
renderer comparison and framework milestones. Do not resume demo-save/UI refinement.

## Current engine systems branch

At `70fc8bf`, fine collision can be prepared on a worker thread and published only
against the same authoritative detail scene. Opaque scene versions distinguish
unrelated/replaced scenes, forks and edits without hashing geometry or copying
voxel payloads. Stale publication preserves current physics. Two version tests,
19 collision tests and strict detail/physics Clippy pass. At `6143e87`, the bounded asynchronous controller and corrected reset/reversal
logic pass its queue/thread/contact tests. The direct ARM64 Android executable at `9cf26a1` passes asynchronous floor
creation, standing contact, removal and falling on OnePlus13. Frame-loop
publication cadence is not yet integrated.

At `94e51eb`, world clones and streaming overrides share immutable chunk payloads.
Changed chunks detach once; subsequent edits reuse their allocation until another
snapshot shares it. Four allocation/behavior regressions and all core tests pass.
See [chunk snapshot evidence](performance/chunk-snapshots.md). No mobile speed or
memory measurement is inferred from allocation identity tests. Android APK build/signature/alignment and indirect-light functional/lifecycle
checks pass at `7211b7d`; later changes still require their own integration checks.

Automatic detail selection is integrated at `a4d7ba0` after100 worker host tests.
The renderer now updates instance selection without recopying geometry; its11-frame
Vulkan check passes ([evidence](performance/instance-updates.md)). The native adapter at `1abe19b` passes9 Vulkan phases
including perspective/orthographic zoom, edits and renderer recreation; OnePlus13 Android16 also passes1080 frames, all9 phases and HOME/resume. GLM numeric regressions and lead corrections
pass103 detail tests; an additional detail snapshot test passes separately.
Streaming latest-request/reset/reversal corrections at `02fc5d3` pass48 core tests. The first bounded diffuse indirect reference is integrated and its Vulkan
shader passes host plus OnePlus13 off/on/sun/enclosure/HOME-resume checks. Complete
phase preparation/upload takes roughly39–45ms on that phone; scheduling and
quality work remain. See [lighting evidence](performance/indirect-light-engine.md). These features are not part of the already published v0.4.0 APK.

## Current delivery and lighting follow-up

PR9 merged at `1cb468e` after all GitHub host, Android and documentation checks
passed at `4535b69`. Its code at `1abe19b` passes326 workspace tests (3 existing
ignored), strict Clippy, ARM64 APK inspection and a1080-frame physical Android
automatic-detail check including HOME/resume. The currently published v0.4.0
APK predates those systems; a v0.5 prerelease is being prepared.

The follow-up branch `codex/engine-light-scheduling` adds bounded background
indirect-light preparation. GLM's partial controller was recovered by the lead;
Astra repaired its test assumptions. All25 focused async tests pass at `2267ad5`.
The native Vulkan adapter at `da96b8e` presented15 frames while lighting was
pending, then correctly published sun/edit/enclosure results. The combined351-test suite and corrected strict Clippy pass. Production-controller
coverage is92.31% of lines with all27 functions exercised. The v0.5 ARM64 APK
build/signature/alignment and physical OnePlus 13 Android 16 checks pass. The final
capture records all six phases, nine presentations during preparation, HOME/resume
and renderer recreation. The earlier secure-keyguard timeout remains a separate
failed attempt. The APK source is `9854723`; the standalone ARM64 CPU check at
`70a231a` additionally matches every cell/face against synchronous lighting for an
open, closed and reopened enclosure. See [background lighting](performance/async-indirect.md).
These functional checks do not establish full GI/reflections or sustained efficiency.

PR10 merged at `3ad0d27` after all final-revision host, Android and docs CI
passed at `577dff8`. Paid Muse Contributor reviewed the controller, queue lifetime,
Vulkan publication and standalone check without actionable findings; lead owns
executed checks. A fresh local APK rebuild was byte-identical to the `9854723`
artifact, passed signature/16 KiB checks and another six-phase OnePlus 13 run with
HOME/resume and nine presentations during preparation.
[v0.5.0](https://github.com/5omeOtherGuy/Matterweave/releases/tag/v0.5.0) is published
with APK, exact-source manifest and retained lighting/detail/collision evidence.
See [delivery manifest](evidence/v0.5-release.json). This is an intermediate engine
delivery; M2–M6 remain open.

The integration branch is `codex/engine-completion-03` (PR11 draft). Hy4's ray
and comparison prototypes were recovered and corrected by lead; paid Muse repaired
collision cadence; Opus refined the detail topology guard after counterexamples.
Both paid Muse Contributor and Opus 5 are working now. Earlier quota notes are
historical; a transient Muse429 was retried successfully. Lead owns integration
and the connected OnePlus 13. The ten-minute native streaming gate passed
39,761 cycles, 219,832 meshes and 3,615 save/reloads within its declared limits.
See [streaming stress](performance/stream-stress.md).

The frozen `b1f6c67` instrumented
tests and four Vulkan examples passed. Per-object LCOV line union reports
94.62–97.01% across the four engine crates with explicit scope/diagnostics; see
[coverage evidence](performance/engine-coverage.md). This is not Android/shader
coverage or an engine-completion percentage. The stopped temporary coverage
build target was removed; profile/report evidence was retained.

## Engine-03 renderer reference

The bounded authoritative ray-volume pack and Naga shader now execute through
a headless Vulkan probe harness. All 73 renderer tests and strict scoped Clippy
pass. Lead corrected 48 host synchronization hazards, then two Adreno precision
failures; the expanded 30-probe suite now passes host synchronization validation
and physical Android. The [manifest](evidence/2026-09-08-ray-reference.json)
retains source/binary checksum and the complete phone report. See
[ray reference](performance/ray-reference.md) for bounds and numerical tolerances.
This is shader correctness evidence only; the full-image same-quality ray/raster/
hybrid comparison and primary-path selection remain in progress.

The ten-minute streaming run sampled RSS between 7,052 and 16,128 KiB, and battery
temperature rose 28.3 to 37.5 C with Android thermal status reaching 3. These are
headless CPU-fixture observations, not a graphics-efficiency claim. See its
[exact summary](evidence/2026-09-08-stream-stress.json).

## Detail and collision integration — Android check pending

The topology guard now evaluates coarse fills sequentially. Independent fills had
jointly closed a 2×2 tunnel; the reproduced regression and all-axis/negative
variants now pass. Opus records 122 detail tests passing, including safe stepped
wedge coarsening, preserved passages, aligned channels that safely reach Half,
and per-instance edit invalidation. Rough concavities remain conservative;
material-filled channels and full temporal/quality M4 acceptance remain open.
See [guard limitations and costs](performance/detail-local-loss-guard.md).

Production wetland edits now queue collision preparation and poll publication
before each simulation step. Added-solid regions defer publication while occupied;
the workerless fallback applies the same gate. Rejection restores prior journal
entries, and current rigid-body poses protect teleports before physics stepping.
Eleven cadence tests and scoped strict Clippy pass. Publication has no fixed time
bound while an addition is occupied; rendering may lead collision. Native/Android
integration validation of this revision remains pending. See [collision log](performance/logs/engine-03-collision.md).

## Earlier continuation evidence (historical pre-release checkpoints)

Current verified progress (2026-09-08 continuation):

- PR8 remains draft/open. All remote host, Android and docs checks pass at
  `3fe93f1`, including generator3 routes and native replay integration.
  Subsequent save restoration/analysis work is local; PR8 is not merged/released.
- Generator3, seed20260908, composition `dfb9f40519a3c151`:34,716,467 expanded
  occupied cells,8,899,364 unique stored cells,6,192flora/10species and5,721,300
  expanded flora cells. Source meshes47,255,040bytes, within64MiB. These are host
  generated-data results, not mobile residency or performance claims. See the
  [manifest](evidence/full-wetland-generator3.json), including recorded routes.
- Graded paths and source-derived connectivity now pass actual continuous Rapier
  traversal: ground632points/197.46667simulation seconds; elevated37/11.35;
  waterside21-point ground prefix/7.1166673. No jumps or intermediate teleports.
  The accepted0.30m autostep is used. An earlier257-second trial silently missed
  six anchors and was rejected; every retained authored anchor now resolves or
  generation fails. All19 prior source tests and the new waterside gate pass.
  Muse found no source blocker; Astra's waterside coverage finding was corrected.
- Workspace257 normal Rust tests now pass with3 deliberately ignored gates. The
  full-map Runtime gate was then explicitly run with the app suite after the
  restored-respawn correction: all64 app tests pass. Strict workspace Clippy
  and the later scoped app Clippy pass; vendored winit warning is unchanged.
  All173 performance Python tests pass, including offline analysis and phone
  route-report checks. Native0.4 Vulkan capture/replay cancellation also passes.
- Entrance correction validates the actual capsule with a bounded vertical lift.
  Generator3 prevents prior layout journals replaying against changed source.
  Corrupt/old-generator sessions select separate recovery files. Invalid edit
  references/body data now participate in actual recovery selection. Candidate
  source is isolated until collision, player pose, bodies and meshes all load.
  Full Runtime round-trip and rejection tests pass; see
  [restoration evidence](performance/showcase/save-validation.md).
- The generator3 native Vulkan/Xvfb/lavapipe capture passed25 frames,20 valid typed rows,
  six physics bodies, isolated persistence and no validation errors. Menu rendering
  is excluded from GPU completion joins. Source prototype geometry is shared on GPU;
  source LOD remains fixed. No automatic LOD or optimization win is claimed.
- The initial six-species full-map development APK (`fffb3844…`) was built, passed
  ARM64/16KiB alignment and signature checks, and was installed on the OnePlus13.
  Normal chooser/entry, movement, HOME/resume and persisted session were observed.
  The corrected ten-species APK (`35cbd2e6…`, source `7e1143e`) was installed;
  normal entry, movement, two source edits, process reload and HOME/resume rendering
  passed. Captures1800/180rows validate but show only renderer epoch1.
  [Device evidence](evidence/2026-09-08-full-wetland-development.md) retains exact
  conditions and limits. Generator3 development APK `4e47b6d9…` (`0d8225b`)
  installed and launched normally; touch move/jump and separate recovery2 save
  observed. A separate owned clearing fixture passed touch fracture6→29 bodies,
  settling and save. Full64-piece stress and verified grab/throw remain open.
  The completed paired collector used frozen generator2 reference/candidate APKs. Per-trial manifests identify
  the installed build. Full phone routes and shadow/temporal quality remain open.
- Actual app route replay is implemented and independently reviewed. Native Vulkan
  smoke advances it and records terminal cancellation correctly. The phone runner
  verifies complete route endpoints and deliberate touch/HOME cancellation;
  the elevated physical route now passes on the shadow-cache build. Ground and interruption checks remain pending.
- The0.4.0 prerelease candidate at `8d8a3e8` built successfully in61s; APK
  `6f22ba87f7a9e3fbaac1c6763c17dd05f59fda1c4b192de329f3c9efd073faee` passes
  ARM64/16KiB and signature checks. Artifacts are under
  `completion-02/release-candidate-04`. Not installed, merged or released.
  Gradle debug uses Cargo dev/opt-level2/debug0; the earlier release-profile
  label in development evidence was corrected against actual frozen source.
- P02 has three completed pairs across A3/A4; the first two are summarized here. Source, fixture, camera, shadows and
  entrance images match. Reference mean73.551/75.820ms; candidate supported mean
  24.260/24.265ms. A4 has nine history gaps (1.7547% of selected span); its whole
  selected-span mean is bounded above by24.653ms without inventing frame data.
  Candidate CPU and PSS are lower, but skin reaches49.199/49.872C and thermal
  status2 versus reference39.139/39.503C/status0. This is a throughput/thermal
  tradeoff, not a blanket efficiency win. [Analysis](performance/p02-analysis.md).
- A3 rejected trial3 after31 cooling observations, preserving its first pair.
  A4 completed the remaining two pairs with unchanged1C battery/2C skin matching
  and exited with successful cleanup. Profiling overhead, motion/temporal quality and
  sustained final-build workload gates remain pending.
- The separate30/60Hz pacing experiment is preserved on its worker branch,
  unintegrated and unmeasured. Its worker has stopped. Reassess the engine pacing
  portion before adoption; further demo UI/save work is paused following owner
  steering. The lead discarded its own uncommitted grab/throw/break feedback UI
  patch. No measured thermal benefit from a cap is claimed.

Next: advance concrete engine capabilities and their native verification. The
completed matched comparison retains its measured thermal tradeoff. Do not make
more showcase save recovery, authored route refinement or gameplay UI polish a
prerequisite for engine work. Automatic LOD, indirect illumination/reflections,
renderer comparison, streaming completion, second-sample reuse and remaining
M2–M6 gates remain open. PR8 was subsequently merged and the shadow-cache APK
was device-checked and released, as recorded above; these open capabilities were
not completed by that release.

The [completion execution log](performance/logs/completion-execution.md) and
[prior campaign status](STATUS_BEFORE_COMPLETION.md) retain earlier
accepted P00/P01/P02/P03 slices and unavailable measurement gates. Latest published
prerelease is v0.5.0; subsequent engine development is reported separately.

## v0.3 verified Android slice

The [v0.3 plan](V0.3.md) has working implementations of directional shadows,
bounded background terrain/mesh preparation and a six-body breakable arch that
fully fractures to64 pieces. The first APK was installed over v0.2 on the OnePlus13;
terrain/camera/old bodies survived. New touch shadow/sun/detail/reset controls work,
and the beam was fractured on-device (6→29 bodies). Old saves receive default
lighting preferences; changed preferences persist.

The [execution log](../execution_log.md) records exact mixed-model assignments,
submissions, review corrections, board behavior and failed diagnostic hypotheses.
Astra, Opus5 and Muse produced isolated contributions; lead integrated them through
[PR #4](https://github.com/5omeOtherGuy/Matterweave/pull/4). The [board](../tools/coordination/README.md)
passed16 local recovery/ownership checks and its GitHub workflow. Submission is
explicitly distinct from lead acceptance.

Current checks:77 Rust tests pass (29 core,17 explorer,17 physics,14 renderer),
workspace Clippy passes, and a90-frame native app smoke passed edits, interaction,
atomic save/reload, resize and renderer recreation with Vulkan synchronization
validation. ARM64 APK build/signature/16KiB ZIP+ELF alignment pass. An exposed face
in an isolated phone fixture is pixel-identical on/off, including low sun at2048;
coarse terrace-shadow edges remain a quality limit of the finite map.

A controlled 120-second warmup plus 20-minute fixed-quality run completed.
All 61,509 selected presentation intervals were co-observed; mean 19.515 ms,
p95 24.878 ms. All 40 health samples reported severe throttling. The run was
USB powered and entered hot: no causal speed/power comparison with v0.2 is valid.
See the [complete evidence](evidence/2026-09-08-v0.3.md) for exact conditions,
clock/coverage limits and CPU/GPU wall-time meanings. Final travel/reversal and
two resume/relaunch cycles passed; the user's saved scene was restored.
GitHub CI passes; final delivery uses PR #4 and the v0.3.0 development prerelease.

The earlier v0.2 timestamp capture has99.8183% verified interval-duration coverage;
its median/p95/p99 verified intervals were16.580834/16.584323/16.585886ms. Four gaps
cross missing dump histories and are not confirmed stalls. Thermal status reached
SEVERE; battery temperature30.1→40.8°C during its measurement window. These are
recorded observations, not a v0.3 comparison or power/thermal superiority claim.

## v0.2 interactive Android slice

M0/M1 shipped in v0.1. The owner reported that version working on a OnePlus 13;
v0.2 has now been directly exercised over USB on that phone with Android 16/API 36,
LineageOS 23.2-20260818-NIGHTLY-dodge and Adreno 830 Vulkan 1.3.284.
The [v0.2 scope](V0.2.md) advances M2/M3; the full M2–M6 gates are not claimed complete.

Implemented changes:

- Material-preserving greedy chunk meshes, local revision invalidation, cached
  Vulkan buffers, conservative frustum culling and dynamic object draws.
- Connected terrain beyond the original island, bounded 7×7×3 residency, persistent
  edited chunk overrides and v0.1 migration that preserves removed chunks.
- Rapier walking, gravity/jump, exact edited-terrain collision, voxel rigid bodies,
  spring grabbing, throwing, bounded fracture and distant-body preservation.
- Touch walk/flight, object aim indicator, HOME, resettable objects, camera/control
  persistence and atomic combined world/object saves with corrupt-session recovery.
- Android repeat-launch protection, explicit Back handling and a narrow vendored
  winit lifecycle patch for destruction and sequential event-loop recreation.
- Version 0.2.0/code 2 ARM64 APK, optimization level 2, same local debug signing
  certificate as the v0.1 release. Dependency provenance and CI are updated.

## Verification actually executed

See [the v0.2 evidence report](evidence/2026-09-07-v0.2.md). Historical v0.1 evidence
remains in [the original report](evidence/2026-09-07-mvp.md).

| Check | Result |
| --- | --- |
| Workspace behavioral tests | PASS: 22 core, 14 explorer, 11 physics, 4 renderer; 51 total. |
| Formatting / Clippy | PASS: workspace/all-targets, locked, warnings denied for workspace code. One unchanged vendored upstream X11 function-cast warning remains a dependency warning. |
| Greedy/reference geometry | PASS: exact exposed unit-face/material equivalence and winding; identical fixture 27,744 → 6,660 triangles (76.0% reduction). Not a mobile FPS comparison. |
| Physics stress | PASS: 64 stacked bodies over 600 fixed steps; floor support, finite/restorable snapshot. Host test only. |
| Host app smoke | PASS: 30 presented frames, edits, grab/throw/fracture, atomic save/reload, actual resize and renderer recreation on llvmpipe with validation enabled. |
| Renderer cache smoke | PASS: six frames with submitted-buffer replacement/eviction, stale/invalid upload retention, culling, dynamic reuse/growth and teardown; no Vulkan validation errors. |
| Android build / signing / 16 KiB alignment | PASS: locked ARM64 APK; v2 debug signature; ZIP entry and all ELF LOAD segments verified, official zipalign passed. |
| OnePlus 13 install / controls / interaction | PASS: installed over v0.1; touch move/look/jump, grab, throw and eight-piece fracture; reset objects and aim indicator. |
| Device streaming / persistence | PASS: flew beyond original bounds to approximately (-8, 8, -63), placed a voxel, updated APK/reloaded; identical authoritative chunk data, camera and 18 bodies retained. |
| Device lifecycle / orientation | PASS: repeat launch, three Home/resume + Back/relaunch cycles in the same process, both landscape rotations and continued saving/rendering. Rotation overrides restored. |
| Documentation / dependency inventory | PASS; recorded source/licenses/checksums and exact winit patch. |

The public APK and its source/build manifest are delivered via
[GitHub Releases](https://github.com/5omeOtherGuy/Matterweave/releases).
Remote PR host/Android/docs checks must pass before merge under the owner's standing
workflow. The release tag identifies the final integrated source; its manifest
records the exact build revision and APK checksum.

## Limits and next actions

First priority: investigate stationary-scene heat. The current app still steps
physics, rebuilds dynamic meshes and redraws shadows while stationary. Profile
CPU busy time/waits, reuse unchanged work, and evaluate frame caps/idle cadence.
The [benchmark protocol](BENCHMARKS.md) now requires unplugged, cooled, matched
conditions for future efficiency comparisons; charging was a confounder here.

1. Complete the equivalent-quality ray/mesh/hybrid mobile comparison. The retained
   reference mesher and new greedy path provide a correctness baseline, not a final
   mobile renderer selection or Nanite-like LOD implementation.
2. Profile boundary-crossing stalls, collision preparation, uploads, residency,
   frame distributions and sustained thermals under the benchmark protocol.
   Background preparation is implemented; collision publication, uploads, snapshot
   copying and saves remain synchronous. Cancellation is versioned and bounded.
3. Stress the expanded64-piece destruction example and gameplay/editor changes.
   Distant bodies freeze before collision eviction. Saves cap overrides at 512 and
   total bytes at 12 MiB; body/camera restoration validates finite bounded values.
4. Continue from direct shadows into M4 indirect illumination/reflections/detail.
   Finite-map edge quality and nonresident casters remain limitations. No full GI,
   reflection, multiresolution transitions, second sample or broader device coverage.
5. Test real simultaneous multi-finger use, lock/unlock, process-memory pressure and
   additional devices. ADB gestures here are sequential; unit tests cover concurrent
   touch roles, but that is not physical multi-finger validation. Phone Vulkan
   validation layers were unavailable; host validation does not cover its driver.

No sustained FPS, GPU-time, power or thermal superiority is claimed from these
functional device tests. Frame/main counters are wall times; voxel payload and
mesh buffer counters are not process RSS or free GPU memory. Presentation teardown
retains the previously documented Vulkan 1.1 WSI idle fallback.

Project licensing, production signing ownership and store publication remain owner
decisions. This is a GitHub development prerelease, not a store build.

Engine-03 full-image comparison: the initial 12-run Android gate passes at
`bfb65ca`; the expanded 16-run host gate includes orthographic opening removal.
It permits at most 0.05% CPU-proven face-edge ambiguity and zero unexplained
mismatches. One/two edge pixels are recorded on the new fixture, not silently
classified away. The one-cell descriptor regression is corrected; matched CPU
oracle acceptance requires at least one non-excluded hit. Final Android repeat
and combined workspace checks are running. [Protocol](performance/renderer-comparison.md).
