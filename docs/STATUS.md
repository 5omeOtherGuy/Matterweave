# Current status

Updated: 2026-09-07

## v0.3 planning handoff

The [v0.3 delivery and coordination plan](V0.3.md) records the next feature slices
(shadows, bounded asynchronous streaming and a destruction playground), worker
allocation, board authority, bounded reads, acknowledgment/recovery rules and
required startup rehearsals. It is durable project context, not shipped functionality.
The board wrapper, live implementation run and v0.3 device measurements are not
implemented/run. Next: validate the coordination access layer and recovery gates,
establish the phone baseline and shared interfaces, then dispatch bounded workers.

Planning verification: the installed orchestration helper's 11 tests passed;
those checks do not prove the proposed wrapper or process-fencing protocol.
Documentation verification: `python3 tools/check_docs.py` and `git diff --check`
pass for this handoff update. No native code or APK changed in this update.

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

1. Complete the equivalent-quality ray/mesh/hybrid mobile comparison. The retained
   reference mesher and new greedy path provide a correctness baseline, not a final
   mobile renderer selection or Nanite-like LOD implementation.
2. Profile boundary-crossing stalls, collision preparation, uploads, residency,
   frame distributions and sustained thermals under the benchmark protocol.
   Current streaming/meshing/saving is synchronous; no async cancellation claims.
3. Expand the 64-body destruction example and stress gameplay/editor changes.
   Distant bodies freeze before collision eviction. Saves cap overrides at 512 and
   total bytes at 12 MiB; body/camera restoration validates finite bounded values.
4. Start direct shadows and then M4 indirect illumination/reflections/detail work.
   Lighting currently remains direct sun, ambient and fog. No full GI, reflection,
   multiresolution transitions, second game sample or broader device coverage.
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
