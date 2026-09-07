# Requirements and traceability

## Status meanings

**Owner requirement** records an explicit current goal. **Working requirement** is an engineering interpretation or setup convention that the implementation session may refine with evidence. **Research candidate** records an opportunity, not a delivery commitment. All acceptance evidence below is currently outstanding unless [STATUS.md](STATUS.md) says otherwise.

| ID | Requirement | Basis | Evidence required | ADR / milestone |
| --- | --- | --- | --- | --- |
| R01 | Deliver an Android-native engine and sample application. | Owner requirement | Reproducible ARM64 APK, native execution and recorded physical-device run. | 0001, 0003 / M0–M1 |
| R02 | Use voxels as meaningful world/object data with fine-detail capability. | Owner requirement | Queryable voxel data, two resolutions, edits and persistence; not only block-themed meshes. | 0005 / M1–M3 |
| R03 | Prioritize high-end Android fidelity and sustained efficiency. | Owner requirement | CPU/GPU frame times, memory/residency and thermal traces under the benchmark protocol. | 0002, 0012 / all |
| R04 | Investigate useful multicore CPU, GPU, RAM and NPU/accelerator capabilities. | Owner requirement | Device capability inventory and measured adopt/defer conclusions; bounded memory and work scheduling. | 0002, 0013 / M0–M5 |
| R05 | Support multiple genres, environments and 3D/2.5D views. | Owner requirement | Two samples with different cameras and mechanics sharing the same runtime. | 0001, 0011 / M6 |
| R06 | Support procedural worlds and replayable game systems. | Owner requirement | Seeded generation, versioned generator identity, save/load of edits and a reproducible sample. | 0011 / M1, M3, M6 |
| R07 | Enable complex physical interaction. | Owner requirement | Dynamic bodies, constraints, collision queries and interaction with edited geometry. | 0010 / M3 |
| R08 | Provide Nanite-like automatic detail selection with smooth distant simplification. | Owner requirement | Repeatable approach/retreat/zoom captures; silhouette, thin-feature and seam checks; bounded residency. | 0007 / M2–M4 |
| R09 | Provide Lumen-like dynamic indirect lighting and reflections. | Owner requirement | Moving lights/sun, edited enclosure and reflective-material tests with quality/performance evidence. | 0008 / M4 |
| R10 | Provide usable mobile controls. | Working requirement, consistent with Android goal and earlier explicit request | Simultaneous move/look/action, cancel/focus-loss handling, adjustable layout, no desktop-only actions. | 0003, 0011 / M0–M1 |
| R11 | Reuse mature infrastructure where it demonstrably helps. | Working requirement from prior recommendation | Source/license/version record and integration comparison; no blanket rewrite or whole-engine fork by assumption. | 0004 / M0–M2 |
| R12 | Keep world state independent of view-dependent render and simulation approximations. | Working requirement | Camera changes preserve authoritative collision/gameplay; stale jobs cannot overwrite edits. | 0005, 0007, 0010 / M1–M3 |
| R13 | Provide reproducible builds and truthful validation evidence. | Working requirement | Pinned toolchain/dependencies, clean-checkout commands, CI and clearly labeled device results. | 0012 / M0 onward |
| R14 | Keep implementation context in this repository. | Owner requirement | Start instructions, decisions, status, commands and next steps usable without prior chat. | 0001, 0012 / setup onward |
| R15 | Evaluate temporal reconstruction and scalable visibility, materials, shadows and atmosphere. | Working requirement from technical proposal | Baselines and image-stability/performance comparisons before default enablement. | 0009 / M2–M5 |
| R16 | Explore neural lighting, ReSTIR, frame generation and advanced GPU/NPU paths where valuable. | Research candidate | A bounded experiment with total cost, device coverage, image quality and fallback documented. | 0013 / after relevant baseline |

## Prioritization and conflicts

Android-native operation and the reusable voxel product identity take precedence over convenience of a browser or desktop-only implementation. Correct gameplay and stable world state take precedence over camera-dependent shortcuts. Optimize image and simulation quality together with sustained frame time; a feature does not qualify as an optimization if it merely moves work to another processor or hides stalls in averages.

There is no accepted exact FPS target, RAM allowance, voxel size, world size, device floor or UE5 parity claim. Proposed evaluation budgets are in [BENCHMARKS.md](BENCHMARKS.md), where they are explicitly identified as working targets.

## Non-goals for the initial implementation

- Shipping a complete commercial game, all listed genres or an Unreal-scale editor.
- Requiring a browser/Sites backend or reusing the earlier browser demo.
- Requiring ray-only rendering, uniform voxel resolution, cubic art or an enormous dense world allocation.
- Requiring all hardware units to be busy continuously or allocating the phone's full advertised memory.
- Launch support for every Android phone, multiplayer determinism, mod marketplaces or an asset store.
- Full path tracing, NPU rendering, fluids, soft bodies or frame generation as prerequisites for the first useful native milestone.

These boundaries sequence development; they do not prohibit evidence-backed later extensions.
