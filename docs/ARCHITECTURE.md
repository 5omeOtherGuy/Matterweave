# Architecture working proposal

This describes intended boundaries, not existing modules. Accepted outcomes are in [REQUIREMENTS.md](REQUIREMENTS.md); Rust preference, modularity and qualifying reuse are accepted in ADR-0014. Specific implementations remain subject to the active [ADRs](adr/README.md). Introduce boundaries incrementally as working code requires them.

## Rust modules and efficient interfaces

The current foundation proposal is a Rust/Cargo workspace. Use modules/crates with explicit ownership and data contracts, keeping Android, rendering, physics, world data, tools and game rules separable. Reuse adequate implementations inside those boundaries; a bespoke architecture is not a mandate to reimplement all internals. See [component selection](COMPONENT_SELECTION.md).

Prefer stable handles and contiguous/batched data where suitable. Static composition can preserve optimization opportunities; modularity does not require dynamic plugins or per-object virtual calls. Establish foreign interfaces only where needed, with layout, thread, error/panic and lifetime contracts. Expose efficient Rust-facing APIs around necessary unsafe sections. GPU resource retirement and asynchronous world revisions still need explicit correctness mechanisms.

Investigate ash for direct Vulkan and existing Rust Android lifecycle integration. wgpu remains eligible if it meets the actual feature/control/performance requirements. Any new hardware interface starts from an available Android/driver/vendor API and an identified gap or significant expected gain; it does not assume driver privileges or make unsupported features available.

## Runtime responsibilities

| Boundary | Responsibilities | Contract |
| --- | --- | --- |
| Android host | Lifecycle, surfaces, touch/controllers, storage integration, display pacing and device/thermal capabilities | Own platform handles; report events and capabilities to the runtime. |
| Core runtime | Jobs, clocks, identifiers, logging, profiling and resource ownership | No dependence on a particular sample game or Android UI widget. |
| World/content | Voxel objects/chunks, materials, transforms, seeds, edits and persistence | Authoritative state with stable identities and revisions. |
| Derived data | Render representations, acceleration structures, collision proxies and light-cache invalidation | Consume versioned snapshots; discard stale results before publication. |
| Renderer | Visibility, detail selection, residency, geometry, lighting and reconstruction | Render a consistent snapshot with explicit memory/work budgets. |
| Simulation | Fixed-step physics, interactions, queries, character movement and constraints | Gameplay-relevant accuracy independent of camera render LOD. |
| Game framework | Cameras, action input, scene transitions, gameplay state, save data and generation hooks | Reusable services; game-specific battle/inventory rules stay in samples. |
| Tools/samples | Deterministic fixtures, asset conversion, benchmark runner and playable examples | Exercise public boundaries and expose actual costs. |

## World representation

Start with a correctness-friendly CPU voxel representation and explicit world units. Before assets or saves depend on them, document handedness, up axis, transform conventions, voxel indexing, negative-coordinate division and shader matrix conventions. A reasonable engineering starting point is metres, right-handed coordinates and Y up; verify all library adapters rather than assuming matching conventions.

The performance candidate is a sparse hierarchy of compact voxel blocks. Terrain chunks and movable voxel object volumes should be separable so a rigid object's transform can change without rewriting terrain. Block dimensions, occupancy versus signed-distance fields, density precision, material encoding and compression are open experiments. An octree, wider tree, page table or DAG is not mandated by the name of the project.

Authoritative material identity and geometric occupancy must remain independent of transient GPU buffer offsets. Fine color/normal/material detail need not force the same geometric voxel density. Preserve a slow reference query path for testing accelerated traversal and edits.

## Edits and asynchronous work

An edit creates a world revision and identifies affected regions plus dependent neighbors. Geometry, normals, LOD parents, collision and lighting may require different affected regions. GI invalidation can extend well beyond the edited block when an opening changes visibility.

Workers operate on snapshots or clearly synchronized data. A completed job publishes only if its revision and ownership remain valid. Replaced GPU resources are retired after relevant GPU work finishes. Bound outstanding jobs, staging data and old snapshots; chunk thrashing must not create unbounded queues or memory.

Publish collision/world changes with defined simulation semantics. A temporary visual/physics delay must be bounded and visible to gameplay code. Do not silently let stale colliders trap the player or permit falling through recently restored terrain. Save operations must capture a consistent world revision even while edits continue.

## Rendering comparison

ADR-0006 defines the experiment. Candidate A traverses voxel data in compute shaders. Candidate B extracts and rasterizes voxel surfaces. Candidate C combines approaches, for example rasterizing selected surfaces/characters and tracing voxel data for visibility or lighting. Hybrid is the starting hypothesis, not a benchmark conclusion.

All candidates must produce compatible scene depth, normals, material identity and motion information where required. Compare both perspective and orthographic views. Bound screen-space geometric error, material fidelity and internal resolution so a cheaper-looking candidate is not mislabeled as faster at equivalent quality.

GPU-driven culling, indirect work and hierarchical depth rejection are candidates when scene complexity justifies their overhead. Main-camera occlusion must not remove objects still needed for shadows, reflections, GI or physics. Avoid unnecessary full-frame intermediate buffers and CPU/GPU readbacks; measure tile-based GPU bandwidth effects [in the Vulkan guidance](https://docs.vulkan.org/guide/latest/tile_based_rendering_best_practices.html).

## Virtualized detail and residency

Choose LOD by projected error/coverage with hysteresis and a defined transition method. Preserve silhouette/occupancy constraints for small openings and thin features; test topology changes explicitly. Keep a coarse resident fallback while fine pages stream, prioritize visible requests, bound upload work and handle rapid camera reversals. Visibility changes and LOD changes need correct motion/history handling.

Separate asset pages, GPU allocations, CPU decoded data and disk storage in accounting. Android CPU and GPU often compete for shared system resources; counting each allocation category is useful, but do not simply add overlapping reported memory counters and label the result physical usage.

## Lighting and temporal reconstruction

Start with direct lighting and a stable reference. Then evaluate sparse probes/radiance caches with budgeted updates and tracing. Diffuse GI, specular reflections, direct shadows and volumetrics have different sampling/quality needs. Record update latency and energy/stability tradeoffs; a static baked image is not evidence of dynamic GI.

Temporal reconstruction requires prior object/camera transforms, depth, jitter and handling for newly revealed surfaces, edited geometry, particles and changing lighting. Cache validity must follow stable identities and revisions. Transparent materials and moving foliage need explicit policies. Introduce an upscaler only after required inputs are correct.

## Physics and gameplay

Assess suitable Rust physics first, with Rapier an initial candidate. Jolt is eligible only for major workload-relevant advantages after binding and integration costs, under ADR-0014. Generate bounded collision proxies suitable for dynamic bodies and update affected static collision regions. Do not turn every visual voxel into an individual rigid body. Reuse or extend qualifying fracture/connectivity implementations; create project-specific work only for a demonstrated gap or significant advantage. A physics library does not automatically supply voxel destruction.

Use a fixed simulation step with bounded catch-up and render interpolation. Account for determinism limits; reproducible generation does not imply cross-device bit-exact physics or multiplayer lockstep. Separate cheap logical simulation from local detailed physical simulation without changing gameplay-critical outcomes due to camera distance.

Provide action-based input that supports touch, controllers and host debugging. Perspective, orthographic/isometric and constrained 2.5D gameplay should share world services. Seeded generators are versioned functions of seed/configuration; saves retain edits and game state independently from disposable rendering caches.

## Hardware and integration policy

Report features from the actual device/API/driver. CPU worker counts, SIMD, shader precision, subgroup operations, asynchronous compute, RT and NPU workloads need capability checks and end-to-end measurements. Hardware RT does not automatically accelerate an arbitrary voxel data structure; include conversion, acceleration-structure build/refit and synchronization costs.

Use [Android thermal feedback](https://developer.android.com/games/optimize/adpf/thermal) to investigate stable quality adaptation. Avoid competing independent systems each increasing work when they observe spare time. Prefer one budget policy with hysteresis and per-feature priorities. A lower memory profile should evict caches and reduce detail predictably while preserving authoritative game state.

## Potential source layout

The implementation session may adapt this layout. These directories are not yet implemented: `engine/core`, `engine/world`, `engine/render`, `engine/physics`, `engine/game`, `platform/android`, `samples`, `tests`, `benchmarks` and `tools`. Keep build commands aligned with the layout actually created; do not add placeholder modules merely to match this list.
