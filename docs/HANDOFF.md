# Implementation handoff

The entry point for a fresh session: what is true now and what is authorized next.
[STATUS](STATUS.md) owns the dated evidence; where the two differ, STATUS is
authoritative.

## What the project is

Matterweave is a modular native Android voxel engine and game framework, built
primarily in Rust. Android-native delivery is mandatory. The engine is the reusable
product; genre samples validate it. [PROJECT_BRIEF.md](PROJECT_BRIEF.md) owns product
scope and owner priorities; [REQUIREMENTS.md](REQUIREMENTS.md) owns acceptance
criteria; [ARCHITECTURE.md](ARCHITECTURE.md), the [ADR index](adr/README.md) and
[COMPONENT_SELECTION.md](COMPONENT_SELECTION.md) own boundaries and selection policy.

The repository was created empty on 2026-09-07. The setup commits established the
requirement register (now 20 entries) and 13 ADRs; accepted ADR-0014/0015 were added
later, ADR-0016 is a later proposal, and ADR-0003/0004 are Superseded. No engine,
APK, dependency lock or device result existed in those documentation-only commits.

## Current state

The workspace is at version 0.5.0; v0.5.0 is the latest published prerelease, an
intermediate engine delivery rather than a finished engine. The tree includes a Rust
voxel core, fine-detail volumes with automatic selection, an ash/Vulkan renderer with
direct and bounded diffuse indirect lighting, a Rapier physics adapter, a frame
pacer, a bounded audio service (host-tested; its 2026-09-12 device diagnostic fails
at suspension) and a native Android explorer. [README.md](../README.md) lists the subsystems;
[STATUS.md](STATUS.md) gives the verified state, open gates and the live integration
branch. M2–M6 remain open.

## Read in this order

1. [AGENTS.md](../AGENTS.md) — contribution, scope and evidence rules.
2. [STATUS.md](STATUS.md) — verified state, open gates, live branch.
3. [REQUIREMENTS.md](REQUIREMENTS.md) and the [ADR index](adr/README.md) — accepted goals and decision status.
4. [ROADMAP.md](ROADMAP.md) and [MVP.md](MVP.md) — milestone order and the shipped M0/M1 slice.
5. [DEVELOPMENT.md](DEVELOPMENT.md) — real build, test and Android commands.
6. [PERFORMANCE_PLAN.md](PERFORMANCE_PLAN.md), [PERFORMANCE_TASKS.md](PERFORMANCE_TASKS.md) and [SHOWCASE.md](SHOWCASE.md) — active campaign, task assignments and mandatory workload.
7. Inspect current source, branches and worktree changes before trusting any summary, including this one.

## Next authorized work

Use the [engine completion orchestration plan](ENGINE_COMPLETION_PLAN.md) for the
next dispatch order and M2–M6 closure gates. Reconcile historical ownership before
starting workers. Preserve the existing device-playable Voxel Relay sample; repair
the real-backend audio suspend failure and complete remaining engine acceptance.

Continue the engine-completion campaign in [PERFORMANCE_PLAN.md](PERFORMANCE_PLAN.md)
and its [task list](PERFORMANCE_TASKS.md): advance renderer, physics, streaming,
lighting and measured efficiency under the owner's 2026-09-08 direction to finish the
engine and focus on actual engine systems. Open capabilities include the
equivalent-quality ray/mesh/hybrid comparison and primary-path selection, full
Lumen-like indirect lighting/reflections, sustained efficiency and thermal
acceptance, streaming completion and the M6 second sample; [STATUS.md](STATUS.md)
records which gates remain.

The owner requires extensive use of all six model families (see the roster in
[PERFORMANCE_PLAN.md](PERFORMANCE_PLAN.md)) for bounded parallel implementation and
review, with independent Muse/Gemini reviews before the lead's review, testable
worker definitions of done and engineering-log handoffs; the owner has explicitly
overridden conservative-delegation guidance. This direction supersedes earlier
showcase-campaign ordering. Do not repeat the requirements interview or delegate
another plan-writing round in place of code; the plan exists. The lead retains
ownership, integration and verification.

[SHOWCASE.md](SHOWCASE.md)'s dense alien fungal wetland is a mandatory workload, not
optional content, and is the validation workload for engine work. Further demo-save
compatibility, showcase content refinement and gameplay UI polish must not precede
renderer, physics, streaming, lighting and efficiency progress. Efficiency
comparisons follow the [benchmark protocol](BENCHMARKS.md)'s unplugged, cooled,
matched-conditions requirement. Do not treat a planned feature as shipped work.

## Working decisions and prohibitions

- Rust wherever feasible without material detriment; modularity and evidence-led
  reuse are accepted policy ([ADR-0014](adr/0014-rust-modularity-and-evidence-led-reuse.md)).
  The Rust/Cargo/Vulkan native foundation is accepted
  ([ADR-0015](adr/0015-rust-native-foundation.md)). Remaining proposals (ADR-0005–0011,
  0013 and 0016) stay Proposed until their gates are met; accept or revise routine
  engineering choices with evidence instead of waiting for owner approval. Build new
  technology, tools or Rust hardware interfaces only when necessary or significantly
  advantageous, and do not restart the language debate or follow the superseded C++
  foundation proposal (ADR-0003).
- Jolt is eligible only for a documented major advantage over viable Rust physics
  after binding and integration costs; Rapier is the current baseline, not a
  policy-selected winner. Reconstruction candidates such as Arm ASR remain open under
  [ADR-0009](adr/0009-reconstruction-and-scalable-effects.md).
- Android-native delivery is mandatory; a web preview or desktop executable is
  supporting evidence only. The earlier browser/site demo, its scene, memory limits,
  fixed scale and art direction are not requirements. No separate Pokémon project,
  prior personal conversation, copied assets or unrelated-project rules are needed.
- Do not scaffold a second app or restart the completed M0/M1 work. Keep engine
  services separate from game-specific content, and keep Vulkan and physics-library
  types out of gameplay interfaces.
- Preserve unrelated work; do not force-push shared history.
- Owner-only decisions: project license (unselected — do not add, imply or name one),
  purchases, repository visibility and access, release signing ownership and store
  publication.

## Closeout

Run the relevant checks, inspect the diff, and update [STATUS.md](STATUS.md) with
exact accomplishments, commands and results, unresolved risks and next actions.
Update affected ADRs and the [roadmap](ROADMAP.md). Commit coherent changes on a
branch under the active integration authorization and preserve unrelated work.

Report what actually works, what was tested, where the APK and evidence are, and the
next concrete task. Do not present a build-only result as device-tested or a target
as a measurement. If no physical device is available, complete buildable code and
host tests, mark device validation NOT RUN, and give exact reproduction steps. Do not
call a host or emulator result device-verified, or use emulator timing to select the
final mobile renderer; missing hardware is a validation limitation, not a reason to
stop.
