# Current status

Updated: 2026-09-08

## Active engine completion campaign

The full engine objective remains open. Execute [PERFORMANCE_PLAN](PERFORMANCE_PLAN.md),
[PERFORMANCE_TASKS](PERFORMANCE_TASKS.md), [SHOWCASE](SHOWCASE.md), then remaining
M2–M6 requirements. The sole current integration checkout is
`/mnt/bench/matterweave-dev/worktrees/performance-p00`, branch `codex/performance-p00`.
The [board](performance/board.json) records ownership; the prior two-lead arrangement
is retired. This continuation owns phone, integration and delivery.

Current verified progress (2026-09-08 continuation):

- PR8 remains draft/open. Host, Android and docs CI passed at `c6e2317`;
  the missing X11 library was already fixed. Current local source is `7e1143e`;
  newer work has not yet been checked remotely, merged or released.
- The integrated 128m map contains 34,864,520 instance-expanded occupied cells,
  8,876,712 unique stored cells and 6,221 plants across four mushroom and six other
  flora archetypes. Flora contributes 5,908,389 expanded cells. Source meshes total
  45,328,920 bytes. Generator 2, seed 20260908, composition hash `f458591e7b345546`.
  These are generated-data/host results, not mobile residency or performance claims.
- The final map's 19 source tests pass, including every continuous route segment's
  flora clearance, actual horizontal voxel-corner radius and carve-aware terrain.
  Independent Muse/Gemini reviews found the original radius/segment errors; both
  reviewed the corrections without substantiated new findings. Actual continuous
  Rapier traversal is assigned to an isolated bounded worker; phone routes remain open.
- The workspace suite passed 238 tests, with one expensive runtime test explicitly
  ignored. Running that full-map test separately passed load, standing, jump, source
  edit/collision, save and reload in 16.45s on the final ten-species map. Strict
  Clippy passes for workspace code (one upstream vendored winit warning remains).
  Python tooling has 130 passing tests; the historical 226 count double-counted tests.
- Entrance correction validates the actual capsule with a bounded vertical adjustment.
  Corrupt/old-generator saves retain their bytes and select separate recovery files.
  Invalid edit references/body data rejected later in runtime still need recovery
  coverage. Generator 2 prevents old layout edit journals replaying against new plants.
- The final native Vulkan/Xvfb/lavapipe capture passed 25 frames, 20 valid typed rows,
  six physics bodies, isolated persistence and no validation errors. Menu rendering
  is excluded from GPU completion joins. Source prototype geometry is shared on GPU;
  source LOD remains fixed. No automatic LOD or optimization win is claimed.
- The initial six-species full-map development APK (`fffb3844…`) was built, passed
  ARM64/16KiB alignment and signature checks, and was installed on the OnePlus13.
  Normal chooser/entry, movement, HOME/resume and persisted session were observed.
  The corrected ten-species APK is rebuilding from `7e1143e`; its installation,
  full route/edit/water/destruction checks and matched performance evidence are pending.

Next: verify the final APK and native route replay, correct any blockers, then
exercise routes, edit/reload, water/destruction and lifecycle on the phone. Preserve
all density targets and measurement matching limits. Complete final checks, merge
accepted work and publish a tested prerelease with durable evidence. Full automatic
LOD, indirect illumination/reflections, second-sample reuse and remaining M2–M6 gates
remain open. The full engine objective is active.

The [completion execution log](performance/logs/completion-execution.md) and
[prior campaign status](STATUS_BEFORE_COMPLETION.md) retain earlier
accepted P00/P01/P02/P03 slices and unavailable measurement gates. Latest published
prerelease remains v0.3.0; development builds are reported separately.

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
