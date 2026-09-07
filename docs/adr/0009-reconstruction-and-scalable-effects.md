# ADR-0009: Temporal reconstruction and scalable effects

- Date: 2026-09-07
- Status: Proposed
- Basis: Engineering recommendation supporting fidelity and efficiency; not a selected dependency.
- Requirements: R03, R08, R09, R15

## Context

Fine geometry and lighting compete for a mobile frame budget. Reusing temporal information and selectively scaling effects may increase useful quality, but reconstruction introduces its own cost and artifacts.

## Decision

Provide correct depth, prior/current camera and object transforms, motion vectors, jitter and change/disocclusion handling early. Evaluate Arm ASR's generic Vulkan integration against native rendering and a simpler baseline once these inputs work. Measure reconstruction cost and quality, including moving or destroyed voxels.

Evaluate cached/scalable shadows, material/detail streaming, foliage lighting and volumetric atmosphere after the baseline. Virtual shadow-map techniques are candidates, not a mandatory full Unreal-style subsystem. Coordinate dynamic resolution, detail and effect budgets under one stable policy.

## Alternatives

Native-resolution rendering may win for some workloads. Simpler spatial reconstruction can serve a baseline. Aggressive temporal accumulation may look good in stills but fail in movement. Frame generation is a separate later research track, not a substitute for simulation/input responsiveness.

## Consequences

Motion correctness, edited surfaces, particles, transparency and new visibility require explicit handling. Cached shadow pages may invalidate often in a destructive world. Avoid allowing several independent quality controllers to oscillate or obscure benchmark comparisons.

## Validation

M4 tests motion, edits, camera cuts and disocclusion with matched captures. M5 compares full rendering plus reconstruction/effect costs at stated resolutions. Adopt features only after an end-to-end gain or documented quality benefit.

## References

[Arm ASR generic library](https://github.com/arm/accuracy-super-resolution-generic-library), [benchmark protocol](../BENCHMARKS.md), [advanced research](0013-advanced-hardware-and-research.md).
