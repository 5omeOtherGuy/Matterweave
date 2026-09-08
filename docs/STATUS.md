# Current status

Updated: 2026-09-08

## Next campaign: performance and dense alien showcase

Execution has started in the separate `codex/performance-p00` checkout from
`7556204`; see the [sole campaign board](performance/board.json) and
[P00 execution evidence](performance/p00.md). The existing [P00–P07 tasks](PERFORMANCE_TASKS.md)
and mandatory [showcase](SHOWCASE.md) remain the scope, not a replacement plan.

- P00: lead-written compact board and stateless patch-handoff validator implemented.
  Real Pi cancellation retained partial work; replacement rejects stale attempts.
  Independent Muse/Gemini reviews preceded lead inspection; 31 host validator tests
  pass after a reproduced artifact-type correction. Six exact route selection checks
  pass; selection alone is not model task qualification.
- P01: corrected typed CSV v2 code slice and profile validator accepted after
  independent Muse/Gemini reviews and lead verification. Python checks:31 handoff,
  65 conditions and130 profile tests. Real Android captures include1000 rows/epoch1
  and581 rows/epochs1–3; host replay fixture has10 tests. Full app/physics replay,
  some edit/streaming/collision counters and full instrumentation overhead remain
  incomplete. [Three short recorder pairs](evidence/2026-09-08-p01-overhead.md)
  now completed: ON mean-interval differences +0.032/+0.700/+0.673%, thermal0 and
  no unsupported SF gaps. Process CPU is a whole-process rate, not per-frame work.
- P02: CPU interpolated-geometry cache and transactional GPU upload invalidation
  implemented. Eight geometry and three upload-state tests constrain motion,
  sleeping rotations, wake/fracture/restore, empty geometry and renderer recreation.
  Combined native candidate `cc46873`:180 workspace tests, Clippy/fmt pass;
  90-frame normal Vulkan lifecycle smoke passes. Serial independent Muse/Gemini
  reviews/corrections completed and source is accepted for device evaluation.
  Real phone normal-mode2190 rows/epochs1–2 include sleeping cache hits and a
  resume upload without CPU rebuild. Matched APK comparison558274 stopped before
  capture: phone was colder than its old reference, not within matching limits.
  Establish a fresh cold reference; no qualified optimization win is claimed.
- P03: reviewed foundation and flora source merged through PR6 (`524f29eb`) and
  PR7 (`c71306d`), with host/Android/docs CI passing. Frozen flora64661cf/267c2c8:
  84 plants/six types,52686 expanded flora cells. An isolated opt-in
  [native gallery](performance/p03/native-gallery.md) now supports `flora source`,
  caches once and preserves the user-world path by construction. Coarse flora LODs
  are rejected. Actual phone flora rendered85 instances/120582 triangles and
  survived HOME/resume (2194 captured rows/epochs1–2); world bytes unchanged.
  Close-range anatomy, water/lighting, traversal/collision and cost/peak-memory
  acceptance remain open. Desktop has the
  nonoverlapping finest-source collision-adapter lane; lead owns app/phone/merge.
- Device: current save backed up and installed baseline APK hash verified. Owner
  enabled wireless debugging and unplugged USB; charging is confirmed off. Idle
  cooling observations are not app performance. First capture was rejected for a
  foreground/surface-identity problem. A [corrected short baseline](evidence/2026-09-08-performance-baseline.md)
  completed: 5,477 co-observed intervals, median16.583 ms, p95/p99≈33.17 ms;
  all eight sampled thermal statuses0. One baseline is not an optimization comparison
  or overhead qualification. User save restored; lead retains device/settings ownership.
- Owner authorized up to $9.99 existing Hy4 credit, capped by actual $9.985349664
  remaining. No key-level limit is configured; enforceable no-top-up execution remains
  pending. No purchases, top-ups or paid fallback authorized.

No mobile optimization win or complete native showcase is shipped. Next: finish
matched frozen-APK P02 comparisons and native flora quality/collision/cost gates. A separate `codex/p02-wsi-probe` diagnostic based on
P01 (`eb045e5`) logs16 captured draws without changing rendering policy. This is
not a performance comparator. Own-app simpleperf was denied by Android security;
no suggested security-property change or escalation was applied. First WSI trial's
post-reinstall save verification failed; explicit stop plus synchronous shell-write
restoration produced two byte-identical reads of the original. Its failed trial/log
is retained; corrected trial02 restored APK/save successfully. The subsequent
native functional trial also restored the save. Paired supervisor558274 subsequently
stopped after31 rejected observations (battery26.3°C/skin26.078°C versus old
31.3°C/31.483°C reference); no comparison capture launched. Final original-save
hash verified. Phone is stopped on the P01 baseline APK; no phone supervisor remains. Android build initially reused stale physics
metadata from the older probe's shared target. Scoped Android workspace-package
cleanup (no active build) and fresh rebuild passed; avoid cross-worktree targets.
Remaining P01/P04–P07 and full-map/water/lighting/density gates are not closed.
Raw overhead artifacts remain on `/mnt/bench`; remote artifact publication remains
an explicit gap. Source commits/repro commands and small summaries are durable here.

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
