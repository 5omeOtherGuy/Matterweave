# Research and reuse assessment

Assessment date: 2026-09-07. Sources below are primary project/vendor documentation or author research. This is a decision aid, not a Matterweave benchmark or an exhaustive engine survey. Verify current source, supported versions and licenses before adoption. None of these projects is bundled or pinned as a dependency in the setup.

## Existing engines and voxel projects

| Candidate | Relevant verified capability | Assessment for Matterweave |
| --- | --- | --- |
| [Google Filament](https://github.com/google/filament) | Real-time physically based renderer with explicit Android support and a focus on efficiency. | Strong rendering baseline/reuse candidate. It does not itself provide the requested complete voxel/game framework; investigate the cost of custom traversal, lighting and edit integration. |
| [Godot Voxel Tools](https://voxel-tools.readthedocs.io/en/latest/) | Voxel terrain tooling built around Godot. | Useful existing-framework alternative and reference for terrain/editing workflows. Measure its actual Android path before making fidelity/performance claims. |
| [Luanti](https://github.com/luanti-org/luanti) | Established voxel game-creation platform with an Android build path. | Useful world/game framework reference. Its existing design and block-oriented content model are not automatically the best foundation for this project's fine-detail renderer. |
| [VoxelHex](https://github.com/Ministry-of-Voxel-Affairs/VoxelHex) | Rust/wgpu sparse voxel-brick tree with GPU ray tracing and mixed-resolution representation. Its README lists lighting and landscape/loading work on the roadmap. | Inspect algorithms and selective reuse. Do not assume a complete, production-ready Android lighting/streaming engine. Account for Rust/FFI/backend integration if retaining a C++ foundation. |
| [voxel-rs](https://github.com/tim-oster/voxel-rs) | Rust/OpenGL sparse voxel-octree ray tracer. | Algorithm and benchmark reference. An Android/Vulkan port and its performance are separate work, not demonstrated by the desktop project. |
| [Unreal Lumen on Android](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-lumen-global-illumination-on-mobile-in-unreal-engine) | Epic documents experimental Lumen on selected high-end Android devices using the desktop renderer and Vulkan SM5, with and without hardware RT. | Valuable visual/integration benchmark. Epic flags significant cost and experimental status; avoid either claiming universal support or dismissing Android Lumen as impossible. |

Earlier screening also considered Unity, Defold and Axmol as general/mobile engine alternatives, and Smash Hit/Teardown-era custom-engine work as inspiration. None was selected or benchmarked for Matterweave. If the shortlist proves unsuitable, investigate these directly rather than relying on an inherited ranking. Relevant starting points are [Defold](https://defold.com/), [Axmol](https://github.com/axmolengine/axmol), [Unity](https://unity.com/) and [Dennis Gustafsson's engine development blog](https://blog.voxagon.se/). These additional links are follow-up leads, not fresh capability assessments.

## Current comparison conclusion

The recommended experiment is a bespoke voxel core with selective infrastructure reuse. This gives ownership over representation, edit propagation and Android workload budgets while avoiding unnecessary rewrites. It is an engineering hypothesis: no evidence currently establishes that Matterweave will outperform an existing engine. A full-engine foundation remains a legitimate outcome if focused integration evidence favors it without abandoning the accepted goals.

Evaluate total development/integration cost, platform support, editing, rendering, physics, authoring tools, dependency maintenance and runtime behavior. One static screenshot or desktop FPS figure cannot resolve that choice. See [ADR-0004](adr/0004-build-and-reuse-boundary.md) and [ADR-0006](adr/0006-rendering-path-selection.md).

## Concrete infrastructure candidates

| Candidate and source | Why investigate | What remains to validate |
| --- | --- | --- |
| [Android Vulkan guidance](https://developer.android.com/games/develop/vulkan/overview) | Android's recommended low-level graphics direction supports the proposed native renderer. | Minimum feature profile, drivers, shader path and exact build tools. |
| [Android Game Development Kit](https://developer.android.com/games/agdk/overview) | Native lifecycle/input integration, frame pacing and related game libraries. | Select needed components and record their exact versions/integration. |
| [Vulkan Memory Allocator](https://github.com/GPUOpen-LibrariesAndSDKs/VulkanMemoryAllocator) | Established allocation infrastructure for Vulkan. | Allocation strategy, budget reporting, transient peaks and chosen release. |
| [Jolt Physics](https://github.com/jrouwe/JoltPhysics) | C++ physics with Android ARM64 support, rigid bodies, constraints and multicore-oriented design. | Android build, scheduling, voxel collider/fracture integration and actual workload cost. |
| [Arm ASR generic library](https://github.com/arm/accuracy-super-resolution-generic-library) | Mobile-oriented temporal upscaling with Vulkan integration and documented motion/depth inputs. | Total frame savings, output quality and handling of moving/edited voxels on the target GPU. |

These components solve different layers. Filament is not a replacement for Jolt; VMA is not a streaming architecture; Jolt does not automatically implement voxel destruction; ASR does not create source geometry. Proposed dependency choices appear in the relevant ADRs.

## Frontier graphics references and limits

**Nanite-like geometry.** Epic describes automatic detail selection and fine-grained streaming of compressed geometry. Matterweave should investigate the analogous screen-space detail and residency goals using its own editable representation. [Nanite overview](https://dev.epicgames.com/documentation/unreal-engine/nanite-virtualized-geometry-in-unreal-engine).

**Actual mobile Nanite evidence.** Arm's June 2026 Mori investigation used a Vivo X200 Pro with an Immortalis-G925 GPU and a modified UE 5.5.2 desktop-renderer path. It reports substantial optimization and material/animation restrictions. This supports feasibility of an advanced mobile experiment, not general support or a predicted Matterweave frame rate. [Arm's investigation](https://developer.arm.com/community/arm-community-blogs/b/mobile-graphics-and-gaming-blog/posts/mori-to-nanite-billions-of-triangles-on-mobile).

**Lumen-like illumination.** Epic's technical description uses tracing plus scene/light caching. Dynamic diffuse probes are a related research family. Matterweave's probe/cache proposal is an independent engineering candidate; the NVIDIA reference does not establish Android NPU/GPU compatibility or performance. [Lumen technical details](https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine), [DDGI algorithms](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/Algorithms.md).

**Temporal reconstruction.** Arm ASR derives from FSR 2 and offers a custom-engine integration. Its input requirements and processing cost must be included in the experiment. Correct motion/history is part of the renderer architecture, including when surfaces change. [ASR implementation](https://github.com/arm/accuracy-super-resolution-generic-library).

**Memory traffic and thermals.** Vulkan's tile-rendering guidance describes bandwidth-sensitive render choices. Android exposes thermal feedback for adaptation. These support profiling and budget control, not assumptions that one pass layout or fixed worker count is optimal everywhere. [Vulkan tile rendering guidance](https://docs.vulkan.org/guide/latest/tile_based_rendering_best_practices.html), [Android Thermal API](https://developer.android.com/games/optimize/adpf/thermal).

**Native packaging.** Android's native page-size guidance is part of selecting/building the complete toolchain and dependencies. Inspect the produced libraries/APK rather than assuming all transitive libraries comply. [Android page-size support](https://developer.android.com/guide/practices/page-sizes).

## Research tracks without adoption decisions

Neural radiance caching, ReSTIR-family sampling, hardware ray queries, variable shading rate, asynchronous compute, mesh-shader paths where supported, frame generation and NPU inference are candidates for later workload-specific evaluation. A frontier label is not evidence of a mobile benefit. Do not select an API simply because a device advertises an accelerator; inventory the actual runtime/driver interfaces and compare transfer, preprocessing, synchronization, quality and thermal cost.

For each new claim, add a primary source, a narrow applicability statement and a local experiment when the claim affects architecture. Use [ADR-0013](adr/0013-advanced-hardware-and-research.md) to record adoption/defer/rejection rather than expanding the baseline indiscriminately.
