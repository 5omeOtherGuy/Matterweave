# Implementation handoff

## Instruction to the next session

Take over Matterweave from the current repository branch. Use this repository as the complete project context. Orchestrate implementation with available subagents where useful, preserve accepted requirements and decisions, resolve proposed technical choices using focused experiments, and advance into verified native Android development. Keep the repository's status and decisions current so another session can continue without chat history.

Do not spend the session repeating the initial requirements interview or producing another plan in place of code. Read the live repository first: this handoff may predate implementation commits.

## Current implementation entry — 2026-09-07

For the next release, read the [v0.3 delivery and coordination plan](V0.3.md).
It preserves the planned feature slices, multi-model assignments, board ownership,
bounded-context protocol and startup verification gates. v0.3 implementation and
the board access wrapper have not started; do not confuse this plan with shipped work.

The M0/M1 MVP shipped, and v0.2 advances M2/M3; see [v0.2 scope](V0.2.md). Start from [STATUS](STATUS.md), the
[development guide](DEVELOPMENT.md), [MVP scope](MVP.md) and
[component record](DEPENDENCIES.md). Code is in `crates/matterweave-core`,
`crates/matterweave-render`, `crates/matterweave-physics` and `apps/explorer`; Android packaging is in `android`.
The first backend is ash/direct Vulkan. Host tests and smoke execution pass and
an ARM64 APK is built/inspected. The owner reported v0.1 working on a OnePlus 13; consult STATUS for subsequent attached-device tests.
The remaining sections preserve the original starting context; they are not a
request to scaffold a second app or restart completed M0/M1 work.

## Starting state of this handoff

On 2026-09-07 the GitHub repository was empty. The initial setup established requirements and 13 ADRs. The subsequent Rust policy update adds accepted ADR-0014 and proposed ADR-0015, supersedes ADR-0003/0004, and brings the requirement register to 20 entries. **There is no engine, APK, demo, dependency lock or device result in these documentation commits.** Consult [STATUS.md](STATUS.md) and Git history for subsequent changes.

## Essential context

Matterweave is the engine/framework product. Android-native delivery, voxel-based worlds, high fidelity, hardware efficiency, reusable game systems, dynamic lighting and automatic fine-detail management are central. Complex physics is a strategic capability. Genre examples include creature-collecting RPGs, roguelikes, exploration, tiny worlds, 3D adventures and 2.5D games.

Rust is now the owner's explicit default wherever feasible without detriment. Modularity, qualifying reuse and maximum useful speed/efficiency govern selection. Build new technology, tools or Rust hardware interfaces when necessary or significantly advantageous. Jolt is eligible only if major advantages justify it. Read [ADR-0014](adr/0014-rust-modularity-and-evidence-led-reuse.md) and [component selection](COMPONENT_SELECTION.md); do not restart the language debate or follow the superseded C++ proposal.

Earlier browser/site demo constraints were explicitly replaced. The former scene is neither an engine dependency nor the required art direction. No separate Pokémon project or earlier personal conversation is needed. See [PROJECT_BRIEF.md](PROJECT_BRIEF.md) for the complete scope history.

## Read and act in this order

1. Read [AGENTS.md](../AGENTS.md), [STATUS.md](STATUS.md), [REQUIREMENTS.md](REQUIREMENTS.md), the [ADR index](adr/README.md) and [ROADMAP.md](ROADMAP.md).
2. Inspect current source, branches, worktree changes, checks and tool/device availability. Preserve unrelated work. Record relevant environment facts without secrets.
3. Resolve M0's Rust toolchain, Android shell, minimum capability profile and pinned dependency revisions. ADR-0015 proposes Cargo/NDK/Gradle, ash/Vulkan and existing Rust Android integration under accepted ADR-0014. Assess viable components before substantial custom work. Accept or revise engineering proposals with rationale; do not block implementation awaiting owner approval of routine choices.
4. Build the native application shell and instrumentation. Include touch input, lifecycle behavior and capability reporting from the start.
5. Deliver M1's small queryable/editable voxel world using the simplest adequate renderer. Preserve a baseline while preparing M2's ray/mesh/hybrid comparison. Do not implement three production engines before the first native sample works.
6. Continue through authorized milestones with useful verification. The roadmap is ordered delivery work, not a requirement to stop after M1 or a promise to finish every research track in one session.

## Working architecture, not a completed selection

- Rust/Cargo core and Vulkan renderer, initially investigating ash; a thin Android shell using the NDK and appropriate existing Rust integration.
- Own the architecture, contracts and world/renderer integration while reusing adequate implementations. Build or extend components only under the accepted necessity/significant-advantage policy.
- Investigate sparse voxel blocks, separate movable object volumes and multiresolution representations.
- Compare compute voxel traversal, rasterized voxel surfaces and a hybrid using the same scenes and quality criteria.
- Investigate suitable Rust physics such as Rapier. Jolt requires major advantages under ADR-0014. Arm ASR remains a reconstruction candidate subject to the same integration criteria.
- Investigate probe/cached dynamic GI with selective tracing and optional hardware RT paths.
- Plan capability-based quality profiles and measured CPU/GPU/NPU scheduling, memory budgets and thermal adaptation.

Rust preference and the selection/modularity policy are **Accepted** in ADR-0014. Specific implementation choices in ADR-0015 and the other active proposals remain **Proposed** until their gates are met. Exact shader toolchain, block size, voxel surface representation, device floor and dependencies are unresolved. ADR-0003/0004 are historical and superseded.

## First handoff-worthy implementation result

A clean-checkout build produces an ARM64 Android APK. A real voxel scene can be explored with touch input and queried/edited, with stable app lifecycle behavior and performance counters. Host tests cover core data correctness. The repository contains actual build/install commands and an honest test report.

If no physical Android device is accessible, still complete buildable code, host tests, APK packaging, static/native validation and available emulator checks. Mark physical-device validation **not run** and provide exact reproduction steps. Do not call the result device-verified or use emulator timing to select the final mobile renderer. Independent implementation work should continue.

## What requires clarification versus research

Routine engineering selections, initial sample mechanics, test assets and experimental budgets can be chosen and documented by the implementation session. Missing hardware is a validation limitation, not an excuse to avoid building. Project licensing, purchases, access/visibility changes, release signing ownership and store publication are owner decisions when they become necessary. See [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).

## Closeout expectations

Commit coherent changes using the active task's integration authorization. Run relevant checks and inspect results. Update status, ADR statuses, dependency/toolchain records, milestone evidence and outstanding issues. Report what actually works, what was tested, where the APK/evidence can be found, and the next concrete task. Do not claim the entire reusable engine is finished merely because a static scene renders.
