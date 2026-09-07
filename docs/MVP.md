# First playable Android slice

The current implementation targets M0 and M1: an ARM64 native Android application
with a reusable Rust voxel core, direct Vulkan rendering through ash, and a small
original procedural exploration sample. It establishes a baseline for the larger
engine described in [the roadmap](ROADMAP.md).

## Acceptance

- Explore seeded, queryable voxel terrain using simultaneous touch movement,
  look and edit controls. Use a free-flying camera for this first sample.
- Remove and place voxels using a reference world ray query. Save and reload
  authoritative edits in private application storage.
- Rebuild derived visible surfaces after edits. Keep material and world revisions
  independent of the camera and renderer.
- Handle Android pause/resume, cancelled input, native surface destruction and
  recreation. Display real workload counts and CPU diagnostics.
- Build an ARM64 development APK from a clean checkout; inspect signing, manifest,
  ELF alignment and uncompressed native-library ZIP alignment for 16 KiB pages.
- Verify world boundaries, negative coordinates, ray behavior, save validation,
  edit persistence, mesh winding, touch roles and input cancellation on the host.
- Record native host smoke results separately from Android-device results.

## Scope boundary

The initial camera is an exploration tool, without a collision/rigid-body claim.
Direct sun, ambient shading and fog form the visual baseline. Lumen-like indirect
lighting, fine-detail LOD, streaming, physics and the second game sample remain
subsequent milestones. A built APK alone does not satisfy physical-device gates.

## Implementation ownership

The lead integrates the Cargo workspace, Android tooling, CI, decision records
and verification. Independent native subagents implement the voxel core,
explorer application, and ash renderer in disjoint directories. The shared
contracts use plain voxel/material, vertex, ray-hit and revision data. No Vulkan
types enter the world-data API. Runtime platform and GPU lifetimes remain in the
Android/application and renderer modules.

See [current status](STATUS.md) for achieved acceptance evidence and remaining work.
