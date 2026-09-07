# Matterweave

**A native Android voxel engine and game framework for detailed, interactive worlds.**

Matterweave aims to combine fine geometric detail, dynamic lighting and complex physics with efficient, sustained performance on modern Android hardware. It is intended to support creature-collecting RPGs, small worlds, roguelikes, exploration, 3D adventures and 2.5D games through reusable engine systems.

The visual targets include lighting inspired by Lumen and smoothly varying geometric detail inspired by Nanite. These describe desired capabilities, not implemented features or a promise of Unreal Engine feature parity. The engine should exploit useful CPU, GPU, memory and accelerator capabilities while measuring their actual contribution to performance and image quality.

## Current state

**Pre-implementation.** This repository contains the project specification, architecture proposals, research, development milestones and implementation handoff. There is no engine, Android application, playable demo or device benchmark yet. Documentation validation is the only implemented tooling.

## Start here

1. Read [AGENTS.md](AGENTS.md) for development and orchestration instructions.
2. Read the [implementation handoff](docs/HANDOFF.md) and [current status](docs/STATUS.md).
3. Read the [project brief](docs/PROJECT_BRIEF.md) and [requirements](docs/REQUIREMENTS.md).
4. Review the [ADR index](docs/adr/README.md), [architecture proposal](docs/ARCHITECTURE.md) and [research](docs/RESEARCH.md).
5. Implement the next milestone in the [roadmap](docs/ROADMAP.md), following the [development guide](docs/DEVELOPMENT.md) and [benchmark protocol](docs/BENCHMARKS.md).

The repository is intended to be sufficient context for a fresh implementation session. No previous chat, browser demo or unrelated game repository is a required input.

## Architectural direction

The initial working proposal is a native C++ core with Vulkan rendering, Android lifecycle/input integration and selective reuse of established libraries. Matterweave would own its voxel representation, streaming, rendering strategy and integration of world edits with lighting and physics. Sparse voxel blocks, a hybrid rendering path, Jolt Physics and Arm ASR are candidates to validate, not dependencies already selected or integrated.

Android is the product platform. Desktop tools and host tests may support development. A browser implementation does not satisfy the project goal.

## Validate this repository

With Python 3.10 or newer:

```sh
python3 tools/check_docs.py
```

Android build and installation commands will be added when the first application exists. See [open questions](docs/OPEN_QUESTIONS.md) for choices the implementation session can resolve and the few decisions reserved for the owner.

## License

The owner has not selected a project license. Public visibility is not a license grant. No third-party engine code or assets are bundled in this setup; dependency selection must record licenses and provenance. License selection does not block implementing and testing original project code for the owner.
