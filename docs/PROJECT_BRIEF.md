# Project brief

## Product

Matterweave is a bespoke native Android voxel engine and game framework. Its purpose is to make detailed, interactive worlds reusable across many small and medium-scale game designs, with particular interest in procedural generation, replayability and rich systems. The engine should be capable of larger workloads as hardware permits; there is no inherited hard world-size limit.

Example games include creature-collecting RPGs, tiny worlds, different kinds of roguelikes, exploration games, 3D adventures and 2.5D RPGs. These examples motivate flexible cameras, input, simulation, persistence and content systems. They do not require implementing all of these games in the initial engine milestone.

## Owner priorities

1. Android is mandatory and should drive architecture from the beginning.
2. Voxels should underpin the engine's world/object capabilities; the project is not merely a mesh renderer displaying a block-shaped art style.
3. Pursue high visual fidelity and useful fine detail, including on high-end devices.
4. Optimize sustained runtime performance and resource efficiency, including multicore CPU work, GPU work, memory management and investigation of NPU/other acceleration.
5. Support complex physics and interactive worlds as a strategic capability.
6. Include dynamic indirect lighting/reflections inspired by Lumen and automatically varying geometric detail inspired by Nanite.
7. Make the engine reusable for different environments, perspectives and game rules.

## Interpretation of efficiency

The owner's ambition to squeeze the most out of hardware is preserved. The engineering objective is the best useful visual and simulation quality within sustained device budgets. Allocating all physical RAM, saturating every core or routing work through an NPU is not an outcome by itself. The engine must respond to available capabilities, operating-system memory pressure and thermal limits while reporting the choices it makes.

Hardware-specific optimization is welcome when isolated, benchmarked and accompanied by a defined fallback or clearly declared device requirement. High-end hardware is the initial focus; universal compatibility with old or low-end phones has not been requested.

## Visual and simulation ambition

Nanite-like means fine detail close to the camera, automatic simplification at lower projected size, bounded residency and visually stable transitions. It does not mean creating detail absent from source content. Voxels do not require visibly large cubes: resolution, surface reconstruction and materials are implementation choices.

Lumen-like means responsive indirect illumination, color bleeding, interior/exterior lighting and reflections in changing worlds. The target is the capability, not exact Unreal implementation, branding or parity. Cached/probe lighting and selective tracing are current proposals, not final choices.

Complex physics means useful interactions between bodies, characters, constraints and editable environments, with destruction a compelling first demonstration. It does not yet specify a full fluid, soft-body or continuum-material solver. Those systems remain possible extensions rather than implied launch commitments.

## Context evolution and superseded constraints

The discussion began with a small browser/site technology demo: a lush, earthy, serene, semi-alien environment, exploration and mobile controls. The owner clarified an interest in a voxel ray engine, then explicitly replaced the earlier constraints with the native Android engine goal.

Consequently, browser hosting, JavaScript/WebGL, the earlier demo implementation, its memory limits, fixed scene scale and its art direction are not requirements for Matterweave. A similar environment can be a showcase. Touch interaction remains a sensible Android baseline, recorded as a working implementation requirement rather than evidence that every old constraint survived.

No existing engine, language, renderer or library was explicitly selected by the owner during the discussion. The assistant recommended a bespoke voxel core with selective reuse and proposed C++/Vulkan, Jolt, Android game libraries and Arm ASR. These are preserved as proposals with decision gates.

## What success looks like

A developer can obtain the repository, build a native Android sample, explore and manipulate a voxel world with mobile controls, and observe detailed geometry, stable detail transitions, dynamic lighting and physical interactions. The same engine supports a second, materially different camera/gameplay example without a fork of its world or renderer. Results are reproducible and include actual device performance, memory and thermal evidence.

The first implementation milestone is intentionally smaller: a reliable native application with real voxel content, input, lifecycle handling and instrumentation. See [the roadmap](ROADMAP.md) for incremental definitions of done.

## Product boundaries still open

Project license, minimum supported Android/device floor, reference devices, exact visual/performance budgets, first showcase art direction and production editor/scripting scope have not been selected. The [open-questions register](OPEN_QUESTIONS.md) supplies working defaults and says when owner input is actually needed.
