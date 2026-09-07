# Benchmark protocol

This protocol defines controlled comparisons. Initial sustained observations are recorded in [v0.3 evidence](evidence/2026-09-08-v0.3.md); they do not establish comparative efficiency. Numeric budgets here remain engineering targets, not demonstrated capabilities.

## Working evaluation budgets

Begin with a proposed 60 FPS responsive profile (16.67 ms frame interval) and a 30 FPS fidelity profile (33.33 ms). Use an explicitly recorded internal/output resolution, initially approximately 1080p output if suitable for the reference display. Do not silently lower resolution, scene density or effects to meet the target. Frame interval includes scheduling/presentation concerns; overlapping CPU and GPU times are not simply added.

Use a minimum 20-minute sustained workload after a documented warm-up. A useful initial target is p95 frame time within the profile interval and p99 below 1.5 times that interval, with stalls and adaptation events reported separately. These thresholds may be revised from measured evidence; record the reason and retain prior results. Thermal/power measurements unavailable on a device remain unknown, not zero.

Choose explicit resident CPU/GPU, staging, upload and job-queue budgets after M0 inventory and M1 measurements. Do not derive an application memory allowance from the device's advertised RAM alone. Measure process footprint and resource-category accounting, avoiding double-counting shared allocations. Report low-memory behavior and peak usage during regeneration, saves, scene transitions and edits.

## Required fixtures

| Fixture | What it resolves | Include |
| --- | --- | --- |
| Close detail | Fine geometry, traversal/raster cost and LOD quality | A detailed object, thin features, cavities, camera approach/retreat and zoom. |
| Foliage/occlusion | Overdraw, many objects and hidden geometry | Repeated vegetation, opaque and masked variants, a wall/ridge, off-screen shadow/GI contributors. |
| Editable enclosure | Lighting and derived-data invalidation | Open/close a wall or roof; moving direct light; color bleeding and light-leak checks. |
| Moving bodies | Dynamic object cost and collision quality | Independent voxel volumes, constraints, pile/fracture workload and edit-to-collider latency. |
| Streaming reversal | Residency and background work | Fast traversal, camera reversal, forced small budgets, canceled jobs and repeated eviction. |
| Orthographic gameplay | Reuse beyond first-person exploration | Top-down/isometric view, zoom, dense small props and selectable objects. |
| Lifecycle stress | Android integration | Pause/resume, focus loss, resize/surface recreation, interrupted touches and save/reload. |

Version each fixture. Record seed, generator version, asset hashes, world dimensions, finest voxel scale, occupied voxels, object/body counts and camera/input sequence. World scale and counts should come from the actual fixture, not invented performance claims. Begin with small reproducible cases and add complexity to expose a specific bottleneck.

## Comparison procedure

1. Build a release/profile APK with pinned dependencies. Record commit, compiler flags, shaders and toolchain. Run correctness checks separately with debug validation/sanitizers where supported.
2. Record phone model, SoC/GPU, physical RAM, OS/build, graphics driver, API/features, display/refresh rate, battery/charging state, case/cooling conditions, ambient conditions where known and performance/game mode.
   For future thermal/efficiency comparisons, follow the owner’s 2026-09-08 correction: disconnect charging and use wireless debugging where available. Verify actual USB/AC/wireless power state; changing reported battery state does not disable physical charging. Begin from a cooled, recorded temperature and comparable battery level, ambient conditions, case, brightness and refresh policy. If unplugged collection is unavailable, label the run powered and exclude causal efficiency comparisons.
3. Use identical scene, seed, camera/input path, lighting, internal/output resolution and explicit visual-error criteria. Compare geometry fidelity as well as throughput. Native-resolution and reconstructed images are separate configurations.
4. Warm up shaders/caches under a documented policy. Measure cold start, asset preparation and first traversal separately; do not hide preprocessing or acceleration-structure rebuild costs.
5. Run at least three repeatable passes for performance selection when feasible. Alternate candidate order or restore comparable thermal conditions. Keep fixed-quality comparisons separate from adaptive-quality experience tests.
6. Collect per-frame CPU wall/work times, GPU timestamps where supported, presentation intervals, p50/p95/p99, long stalls, memory categories, upload/streaming work, physics cost and thermal/adaptation events. Include tracing/denoising/upscaling/transfer costs in accelerator comparisons.
7. Capture matched images and motion sequences. Review LOD seams/popping, shimmer, thin features, disocclusion, reflection errors, light leaks, GI update latency and ghost trails. A still screenshot cannot establish temporal stability.
8. Record scene/run configuration, raw data location/checksum, summary and decision. Retain failures and limitations. Repeat or broaden testing only to resolve a remaining decision risk.

## Evidence levels

- **Host correctness:** useful for algorithms and serialization; no Android runtime claim.
- **Android build/package:** proves compilation/packaging; no successful launch or performance claim.
- **Emulator functional:** useful for lifecycle and integration; not mobile GPU/thermal evidence.
- **Physical-device functional:** confirms behavior on the recorded device/configuration.
- **Physical-device sustained performance:** supports only the measured workload, device and quality profile.

Use a second GPU family/device when available before claiming portability or broad optimization. A single flagship is an initial reference, not proof across all Android hardware. Missing access must not stop independent implementation, but unresolved measurements remain unresolved.

## Report template

For each experiment record: question; candidate revisions; fixture/seed/assets; environment/device; fixed/adaptive quality settings; exact commands; warm-up/run duration and repetitions; correctness outcome; frame-time/memory/thermal data; matched captures; total integration/runtime costs; unavailable measurements; conclusion (adopt, revise, reject or defer); and ADR/milestone affected.

Store compact reports and manifests in Git when they exist. Store large captures and raw data as durable repository artifacts/releases with identifiers and checksums. Never commit fabricated sample results merely to populate a dashboard.

## Next efficiency investigation

Prioritize stationary-scene cost before adding more expensive effects. The v0.3 app
continues physics, constructs the dynamic-object mesh and redraws scene/shadows
while stationary; no idle cadence or thermal adaptation is implemented. Profile
actual CPU busy time and waits, sleeping/active body counts, mesh rebuilds/uploads,
shadow invalidation and presentation pacing. Compare a fixed frame cap and idle
cadence with current behavior under matched unplugged conditions. Reuse unchanged
object geometry/shadows only with explicit invalidation on simulation, camera,
light, edits and lifecycle changes. Preserve input responsiveness and correctness.
Charging is a confounder, not an explanation that excuses unnecessary engine work.
