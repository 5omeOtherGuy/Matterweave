# Implementation handoff

## Instruction to the next session

Take over Matterweave from the current repository branch. Use this repository as the complete project context. Orchestrate implementation with available subagents where useful, preserve accepted requirements and decisions, resolve proposed technical choices using focused experiments, and advance into verified native Android development. Keep the repository's status and decisions current so another session can continue without chat history.

Do not spend the session repeating the initial requirements interview or producing another plan in place of code. Read the live repository first: this handoff may predate implementation commits.

## Starting state of this handoff

On 2026-09-07 the GitHub repository was empty. This setup establishes requirements, 13 ADRs, architectural proposals, research, milestones, documentation validation and contribution conventions. **There is no engine, APK, demo, dependency lock or device result in the setup commit.** Consult [STATUS.md](STATUS.md) and Git history for subsequent changes.

## Essential context

Matterweave is the engine/framework product. Android-native delivery, voxel-based worlds, high fidelity, hardware efficiency, reusable game systems, dynamic lighting and automatic fine-detail management are central. Complex physics is a strategic capability. Genre examples include creature-collecting RPGs, roguelikes, exploration, tiny worlds, 3D adventures and 2.5D games.

Earlier browser/site demo constraints were explicitly replaced. The former scene is neither an engine dependency nor the required art direction. No separate Pokémon project or earlier personal conversation is needed. See [PROJECT_BRIEF.md](PROJECT_BRIEF.md) for the complete scope history.

## Read and act in this order

1. Read [AGENTS.md](../AGENTS.md), [STATUS.md](STATUS.md), [REQUIREMENTS.md](REQUIREMENTS.md), the [ADR index](adr/README.md) and [ROADMAP.md](ROADMAP.md).
2. Inspect current source, branches, worktree changes, checks and tool/device availability. Preserve unrelated work. Record relevant environment facts without secrets.
3. Resolve the M0 foundation choices sufficiently to build: language/toolchain, Android shell, minimum capability profile and pinned dependency revisions. ADR-0003 currently proposes C++20/CMake/Gradle/Vulkan; ADR-0004 proposes selective reuse. Accept or revise engineering decisions with rationale. Do not block implementation awaiting owner approval of routine choices.
4. Build the native application shell and instrumentation. Include touch input, lifecycle behavior and capability reporting from the start.
5. Deliver M1's small queryable/editable voxel world using the simplest adequate renderer. Preserve a baseline while preparing M2's ray/mesh/hybrid comparison. Do not implement three production engines before the first native sample works.
6. Continue through authorized milestones with useful verification. The roadmap is ordered delivery work, not a requirement to stop after M1 or a promise to finish every research track in one session.

## Working architecture, not a completed selection

- Native C++ core and Vulkan renderer; a thin Android shell using the NDK and Android game libraries.
- Own voxel storage, derived-data coordination, streaming and rendering decisions; integrate mature infrastructure selectively.
- Investigate sparse voxel blocks, separate movable object volumes and multiresolution representations.
- Compare compute voxel traversal, rasterized voxel surfaces and a hybrid using the same scenes and quality criteria.
- Investigate Jolt for physics and Arm ASR for temporal reconstruction.
- Investigate probe/cached dynamic GI with selective tracing and optional hardware RT paths.
- Plan capability-based quality profiles and measured CPU/GPU/NPU scheduling, memory budgets and thermal adaptation.

The associated ADRs remain **Proposed** until their stated decision gates are met. Product goals in accepted ADRs remain binding. Exact shader toolchain, block size, voxel surface representation, device floor and dependencies are unresolved.

## First handoff-worthy implementation result

A clean-checkout build produces an ARM64 Android APK. A real voxel scene can be explored with touch input and queried/edited, with stable app lifecycle behavior and performance counters. Host tests cover core data correctness. The repository contains actual build/install commands and an honest test report.

If no physical Android device is accessible, still complete buildable code, host tests, APK packaging, static/native validation and available emulator checks. Mark physical-device validation **not run** and provide exact reproduction steps. Do not call the result device-verified or use emulator timing to select the final mobile renderer. Independent implementation work should continue.

## What requires clarification versus research

Routine engineering selections, initial sample mechanics, test assets and experimental budgets can be chosen and documented by the implementation session. Missing hardware is a validation limitation, not an excuse to avoid building. Project licensing, purchases, access/visibility changes, release signing ownership and store publication are owner decisions when they become necessary. See [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).

## Closeout expectations

Commit coherent changes using the active task's integration authorization. Run relevant checks and inspect results. Update status, ADR statuses, dependency/toolchain records, milestone evidence and outstanding issues. Report what actually works, what was tested, where the APK/evidence can be found, and the next concrete task. Do not claim the entire reusable engine is finished merely because a static scene renders.
