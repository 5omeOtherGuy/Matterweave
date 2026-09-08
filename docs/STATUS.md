# Current status

Updated: 2026-09-08

## Active engine completion campaign

The full engine objective remains open. Owner steering on 2026-09-08 reaffirms
that the engine is the product. Advance rendering, physics, streaming, lighting
and measured efficiency under [PERFORMANCE_PLAN](PERFORMANCE_PLAN.md) and
[PERFORMANCE_TASKS](PERFORMANCE_TASKS.md). The [showcase](SHOWCASE.md) is a test
workload; further showcase polish and old demo-save compatibility must not delay
remaining M2–M6 engine requirements. The sole current integration checkout is
`/mnt/bench/matterweave-dev/worktrees/performance-p00`, branch `codex/engine-systems`.
The [board](performance/board.json) records ownership; the prior two-lead arrangement
is retired. This continuation owns phone, integration and delivery.

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
including perspective/orthographic zoom, edits and renderer recreation; Android
validation of this revision is pending. GLM numeric regressions and lead corrections
pass103 detail tests; an additional detail snapshot test passes separately.
Streaming latest-request/reset/reversal corrections at `02fc5d3` pass48 core tests. The first bounded diffuse indirect reference is integrated and its Vulkan
shader passes host plus OnePlus13 off/on/sun/enclosure/HOME-resume checks. Complete
phase preparation/upload takes roughly39–45ms on that phone; scheduling and
quality work remain. See [lighting evidence](performance/indirect-light-engine.md). These features are not part of the already published v0.4.0 APK.

## Current integration and next slice

PR9 is open: <https://github.com/5omeOtherGuy/Matterweave/pull/9>. All GitHub host,
Android and docs checks pass at `b1f6c67`; later streaming/numeric/detail-native
commits are undergoing combined verification before final delivery. The current
APK test build starts from `1abe19b`; do not attribute those changes to v0.4.0.

Opus4.8/high's current five-hour allowance is exhausted (next reset2026-09-08
16:20UTC). GLM5.3 Flash/high owns a separate bounded background indirect-light
controller experiment; it is not integrated. Hy4/high reviews the corrected
stream/collision queues. Astra audits counter mismatches in the frozen `b1f6c67`
instrumented coverage report. Host tests and four instrumented Vulkan examples
passed, but the combined percentage remains provisional pending that audit.

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
prerelease is v0.4.0; newer engine-systems development is reported separately.

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
