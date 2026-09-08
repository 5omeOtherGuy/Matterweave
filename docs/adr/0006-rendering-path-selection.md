# ADR-0006: Compare voxel ray, mesh and hybrid rendering

- Date: 2026-09-07
- Status: Proposed
- Basis: Engineering comparison required by the revised voxel-engine goal.
- Requirements: R02, R03, R08, R09, R11

## Context

The owner earlier used the term voxel ray engine, then replaced the earlier constraints with a native Android voxel-engine goal. Voxels remain central. A ray-only pipeline was not re-established as a mandatory condition. Mobile performance cannot be inferred from desktop voxel demos.

## Decision

Build a simple native baseline first, then compare focused prototypes of compute voxel traversal, extracted-surface rasterization and a hybrid. Hybrid is the current hypothesis, not a decision supported by results. Use common world fixtures, camera paths, resolution, material/lighting settings and geometric-error criteria.

Keep depth, normals, stable identity and motion data compatible with lighting and temporal needs. Investigate GPU-driven visibility and work generation only with a baseline that exposes their benefit. Preserve non-camera visibility requirements for shadows/reflections/GI.

## Alternatives

Pure traversal avoids some mesh rebuilding but may be limited by divergence/bandwidth. Surface rasterization uses hardware well for some workloads but carries extraction/update costs. Hybrid adds representation and scheduling complexity. Full hardware ray tracing requires explicit device support and suitable acceleration data.

## Consequences

Experiments must include preparation, rebuilds, uploads and memory, not just steady static-frame timing. Do not build three complete production renderers or maintain unused paths indefinitely. Retain a correctness reference after selection.

## Validation

M2 chooses the primary approach with equivalent-quality physical-device evidence where available, including dynamic edits and orthographic views. Without hardware, selection remains provisional for mobile performance. Record rejected options and revisit triggers.

## References

[Benchmark protocol](../BENCHMARKS.md), [research](../RESEARCH.md), [world data](0005-voxel-world-data.md).

## M1 implementation evidence — 2026-09-07

The first reference rasterizes exposed voxel surfaces directly through ash.
Direct sun, ambient shading, fog and a HUD are implemented. The native host
exercise and APK build establish integration evidence, not a measured mobile
renderer selection. No ray/mesh/hybrid comparison has been performed; this ADR
remains Proposed. See [renderer notes](../../crates/matterweave-render/README.md)
and [STATUS](../STATUS.md).


## v0.3 engineering disposition — 2026-09-08

**Retain provisionally:** ash surface rasterization with cached greedy meshes,
independent shadow visibility, bounded async preparation and optional GPU intervals.
The same physical phone supports functional off/on shadow and1024/2048 comparisons.
Those quality/cost toggles do not compare ray/mesh/hybrid representations.

**Defer final primary-path selection:** no equivalent-quality compute traversal or
hybrid prototype has yet been measured against the raster path. Expanding to three
production renderers would delay this complete Android slice without answering a
bounded experiment. The next M2 experiment remains shared-fixture traversal versus
raster costs, including edits/preparation and orthographic views. This ADR remains
Proposed; neither phone operation nor greedy triangle reduction accepts the final
mobile rendering choice. See [STATUS](../STATUS.md) and the [execution log](../../execution_log.md).

## Bounded GPU ray reference — 2026-09-08

The retained unit-voxel ray pack and shader now pass 30 color/depth probes on host
Vulkan with synchronization validation and on Adreno 830 Android. Initial sync
and mobile precision failures were reproduced and corrected; [evidence](../performance/ray-reference.md)
records numerical tolerances and bounds. This resolves shader-execution feasibility
for a bounded reference, not the equivalent-quality renderer comparison. The
primary path remains unselected under this ADR's gate.

The [full-image comparison reference](../performance/renderer-comparison.md) now has 12 passing Android/host fixture runs, including six complete-world matched images and split shared-depth hybrid occlusion. It establishes bounded correctness at 128×128 with matched simple shading. Combined setup costs and single tiny-fixture draws cannot justify primary-path selection; representative quality, sustained total costs and residency comparisons remain required.
