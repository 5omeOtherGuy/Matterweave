# ADR-0008: Dynamic indirect illumination and reflections

- Date: 2026-09-07
- Status: Proposed
- Basis: Proposed implementation of the owner's accepted Lumen-like lighting goal.
- Requirements: R03, R09, R12

## Context

The desired visual capability includes bounced light, color bleeding, interior/exterior transitions and reflections responding to world changes. Lumen uses several tracing/caching mechanisms; it is not simply one full-resolution ray-tracing pass.

## Decision

Start with direct lighting and a repeatable reference scene. Evaluate sparse probes/radiance caches, budgeted updates and selective tracing for diffuse indirect lighting. Treat specular reflections separately, evaluating screen-space information, fallback representations and selective rays. Keep hardware RT optional until support and total benefit are established.

Define invalidation for moving lights/objects and edited geometry. An opening may change lighting beyond its immediate voxel neighbors. Record update latency, temporal reuse and quality controls explicitly.

## Alternatives

Baked lighting alone cannot meet the changing-world goal, but can be a baseline for static content. Full path tracing is a useful reference/research path rather than an assumed mobile default. Directly adopting Unreal's full lighting architecture carries integration and runtime costs. Voxel cone tracing or other GI methods remain eligible experiments.

## Consequences

Light leaks, stale illumination, disocclusion, glossy noise and cache memory are concrete risks. The voxel representation may help visibility queries but does not make GI free. Diffuse GI, reflections, shadows and volumetrics need separate cost/quality accounting.

## Validation

M4 moves lights, changes sun direction and opens/closes a room; record indirect response, reflection changes, thin-wall leaks, update latency and total cost. Include comparison captures and device evidence where available.

## References

[Lumen technical details](https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine), [DDGI algorithm reference](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/Algorithms.md), [research](../RESEARCH.md).

## v0.3 direct-light foundation — 2026-09-08

Implemented a finite directional depth map through existing ash/Naga components: independent resident-caster visibility, camera-centered texel snapping, receiver-plane corrected 3×3 filtering, edge fade, sun presets, 1024/2048 settings, frame-fence-guarded resource changes and optional completed GPU intervals. Phone and host evidence are in [STATUS](../STATUS.md).

The map covers 128 world units; nonresident casters and fine terrace-shadow edges remain limitations. Dynamic objects and edited terrain feed derived geometry, but no bounced illumination, color bleeding, reflections or temporal reconstruction exist. Direct-light prerequisite only, not Lumen-like outcomes; Proposed. See [renderer notes](../../crates/matterweave-render/README.md).

## Engine shadow reuse — 2026-09-08

The directional map is reused while submitted caster geometry and fitted light projection are unchanged; changes invalidate explicitly, and a failed recording cannot publish a cache hit. Native Vulkan and Android checks exercise reuse and rebuilding. Direct-light efficiency only; GI, reflections and M4 acceptance remain open. See [shadow reuse](../performance/shadow-reuse.md). Proposed.

## First diffuse reference — 2026-09-08

A bounded CPU single-bounce surface cache feeds the Vulkan fragment shader: colored diffuse bounce, sun/edit invalidation and enclosure response through authoritative World DDA. Host validation and OnePlus 13 functional and HOME/resume checks pass; complete CPU preparation/upload costs roughly 39–45 ms on the fixed phone fixture. The path is opt-in, unit-voxel-only and one bounce only; no reflections, temporal reconstruction or full-scene GI acceptance follows. See [implementation/evidence](../performance/indirect-light-engine.md). Scheduling, representation coverage and quality/cost comparison gates stay open; Proposed.

## Background preparation reference — 2026-09-08

A bounded CPU worker prepares source/light-matched volumes in slices while the Vulkan owner presents; 25 scheduling/radiance tests and a native off/on/sun/enclosure check pass, the latter presenting 15 frames while work was pending. Publication retains the frame-fence contract. Reflections, production quality, representation coverage and sustained mobile cost remain open. See [controller evidence](../performance/async-indirect.md).

The background adapter also passes OnePlus 13 Android 16 off/on/light/edit/enclosure and HOME/resume checks, with nine presentations during pending work. PR10 and v0.5 deliver this slice; [evidence](../performance/async-indirect.md) records exact source and artifact checksums. M4 acceptance remains open; Proposed.

## Bounded specular reflection reference — 2026-09-09

An opt-in, bounded single specular bounce runs in the existing Vulkan raster renderer: one secondary ray per reflective fragment, traced through a source volume of at most 64³ authoritative voxel cells with a configurable step budget (hard maximum 512). A ray that leaves the volume, self-intersects or exhausts the budget terminates against the background/fog colour. Nonreflective rendering is the default and bit-identical to the previous output. Mirror strength is per-material, and a fragment's material is the authoritative voxel at the shaded face, so no vertex-format change was needed.

Invalidation is explicit: geometry uploads and shadow-resource replacement disable reflection; publication validates `(epoch, revision, seed)` plus a footprint digest, so an equal-revision scene replacement cannot publish; a rejected publication disables first. The sun is a live per-frame uniform and needs no republication.

Host evidence: 28 predetermined non-edge probes and 346 seeded randomized probes agree with an independent CPU `World::raycast` oracle within 0.5/255 (tolerance 3/255); the nonreflective baseline is preserved within 0.5/255 (tolerance 1/255); recorded images show all seven required responses, including an object outside the camera frustum visible only through reflection; and the real Renderer passes enabling/disabling/editing/resize/recreation with an empty Vulkan validation stream. Device gates are NOT RUN: the shared phone is owner-reserved.

This is one bounded specular capability, not Lumen-like lighting. Glossy reflection, shadowed reflected hits, temporal accumulation, denoising, multiple bounces and reflective non-voxel meshes are deferred. See [reflection evidence](../performance/reflections-engine.md). ADR remains Proposed.
