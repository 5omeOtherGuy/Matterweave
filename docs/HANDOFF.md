# Implementation handoff

**Current worker routing (owner, 2026-09-13, latest):** Codex dispatches tasks
only. Use direct Pi `deepseek/deepseek-flash` (DeepSeek V4.1 Flash) and OpenRouter
`z-ai/glm-5.3-flash` for bulk development. The Claude subscription reset, so
Claude/Opus at **medium** is available again for complex reviews and review
fallback (the owner-designated complex-task role), and a separate read-only Opus
reviewer inspects merged production lighting gaps and returns the next-slice
brief (issue47) without phone/write ownership; do not wait on it or duplicate its
scope. Never dispatch Astra workers. Preserve the global three-worker cap.
Phase B cost/thermal rankings are deferred; functional issue experiments may run
alongside engine development. The issue42 ray-hierarchy experiment finished as
PR #51 (merged `92f0a79`, optional CPU evidence only, GPU/Android/cost gates
OPEN). **Current three workers (owner, 2026-09-13):** this delegated
device/integration owner (`w_150a72bd`) holds the phone and delivered PR #53 and
PR #54, and merged the probe PR #57; it owns the remaining device gates and the
handover below. `w_88f312a3` (`deepseek-indirect-retention`) is probing optional
exact indirect-dependency retention in `indirect.rs` + the wetland attach path
(new test/example/log work; no phone and no shared status). `w_17afb30e`
(`deepseek-ray-gpu`, optional GPU traversal candidate for issue 42) has finished
writing; its review verdict is still pending (`w_e3c16633` review complete,
`w_538111ec` verdict running), and its standalone cross-compiled Android
executable route is feasible for handover without an app debug entry. The probe
author `w_e1c8404b`, the PR #54 repair author `w_8a9409db`, the D4.3 authoring
delivery `w_f653a32c` and the read-only next-slice reviews `w_869fcd5b` /
`w_4c9f3d5e` have completed; do not re-dispatch them.

**Current pickup (2026-09-13):** main is `4a64fe0` (PR #57 probe merge onto the
PR #54 lighting merge `1bdfd6f`, itself on PR #53 `f878e1b` / PR #55 `d37ca73`).
The delegated device/integration owner (`w_150a72bd`) delivered both reviewed
slices: shared mobile settings (PR #53 `f878e1b`) and the wetland proxy
GI/reflection slice (PR #54 `1bdfd6f`, source `28d3055` + docs `3d33c4f` +
wording fix `0e37e84`, device APK `1c857c08…`), then merged the test-only probe
PR #57 as `4a64fe0` after applying its review's comment/log precision
corrections. The phone gate covered stationary convergence (~2.9 s), walk/LOD
churn with GI staying live, a real edit rebuild and reconvergence
(`cells=1801 -> 1802`, `gi=false -> gi=true`), HOME/resume input, the real dense
clearing box with zero `MAX_MESH_PROXY_TESTS`/attach errors, and byte-exact save
plus prior-preference restoration. **Open dense/motion cases:** continuous
moving-cell/body GI continuity stays an open functional requirement (not
Phase B); report gating can hide a fast withdraw/reconverge inside 30 frames;
the dense box is exercised for the real clearing, not arbitrary density up to
`MAX_MESH_PROXY_TESTS = 4 Mi`; the `build_failed` latch suppresses a body-only
retry after a failed attach (disclosed, unexercised on device). No "GI is live
during motion" and no full-GI/M4 claim may be made. `w_88f312a3`
(`deepseek-indirect-retention`) is probing indirect-dependency retention in
`indirect.rs` + the wetland attach path (no phone, no shared docs). The optional
GPU traversal work `w_17afb30e` completed; its review verdict is still pending
(`w_e3c16633` review complete, `w_538111ec` verdict running), and the standalone
cross-compiled Android executable route is feasible for handover without an app
debug entry. Earlier context: PR36 lighting and
PR39 streaming merged earlier with all six checks green each. The delegated
integration/acceptance delivery finished: interaction ANR `c1e8777` merged at
`f0da177` (PR #49); the detail diagnostic phase-7/retreat fixture repair
`8ae9e7d` (`21c3ccc` + `28b841a`) passed independent Opus review `w_62fe4c42` and
the full 14-phase detail gate on the OnePlus 13 at 3168x1440 (three PASS runs,
each with a real HOME/resume, byte-identical reports), so PR #48 merged at
`06975ce`; optional CPU traversal PR #51 merged at `92f0a79` with GPU/Android/cost
gates explicitly open and issue 42 open. PR #53 shared mobile settings merged at
`f878e1b` after two Opus acceptances and a full phone gate (chooser/Wetland/Relay,
left/right x normal/large, adapter-level mute, restart, HOME, sample switch); five
saves and the prior preference absence were restored exactly. The last device
artifact is the accepted #54 APK
`1c857c08bdeb50678f2d634042fb9bbe32aaf406dcb889802c56462eb73fabca`, installed
with the app force-stopped and all five saves restored hash-exact. No thermal
campaign. See the
[detail-48 delivery log](performance/logs/detail48-device-delivery.md), the
[lighting delivery log](performance/logs/wetland-lighting-android-delivery.md), the
[settings delivery log](performance/logs/shared-settings-android-delivery.md),
the [manifest](evidence/2026-09-13-detail48/manifest.json) and
[STATUS](STATUS.md).

**Restart checkpoint:** all workers finished; source commits and outstanding gates are listed in [restart handoff](performance/restart-20260912.md). This checkpoint supersedes older live-worker descriptions below.

**Recovery update, 2026-09-12:** PR #35 merged at `ac1ce444` with all six checks
passing. `/mnt/bench` now has hardware I/O failures in its backing device `sdb`;
do not resume writes or builds there until the drive is repaired and verified.
The live lead checkout is
`/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/integration`
(`engine/recovered-mesh-lighting`). See [recovery state](performance/recovery-20260912.md)
and the [board](performance/board.json) for workers. Original worktrees are preserved.

Terrain Lab (PR #29) and production wetland detail integration (PR #31) are merged and functionally verified on Android. Choose **EXPLORE TERRAIN LAB** for the playable terrain comparison. [Controls and evidence](performance/terrain-lab.md). Shared audio PR #32 merged at `f370990` after repaired Android resume checks and all six CI checks; owner hearing is now confirmed.

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
pacer, a bounded audio service (final lifetime/ownership and pause-intent corrections
pass independent reviews, 49 ARM64 correctness checks and 4 × 20 AAudio lifecycle
cycles; both-sample integration remains open) and a native Android explorer. [README.md](../README.md) lists the subsystems;
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

Owner transferred ongoing development, integration and the phone to this lead.
PRs #28/#29/#31 delivered coarse derivation, Terrain Lab and production wetland
selection. PR #32 is merged with repaired Android resume checks. Continue integrating the preserved destruction and lighting deliveries. D2.2 PR #33
is merged at `c86980b`; its graphics-integrated Android
64-piece/20-cycle diagnostic passed its first physical run, but independent review
found lifecycle/reporting boundary defects. Lead repair `5347e91` passes the corrected-device rerun,
including HOME/resume during phase 4; corrective independent review passed. The owner confirmed audio works and released the phone; Android verification
may proceed, preserving current saves. PR #34 D3.1 is merged at `16ad342`; preserve the contributor's live D3.2
worker `w_7ea6fc6d`. Reconcile Pi and Git before dispatch.
Do not start duplicate implementation or treat host checks as device completion.


The follow-up delivered opt-in landscape comparison fixtures and a
[reference reuse assessment](performance/virtual-matter-and-lay-of-the-land.md).
Host default/landscape runs pass (16/24); Android execution is still unrun and
now belongs to this lead's device schedule. See the current STATUS entry and
[review evidence](performance/reviews/landscape-followup.md). All runtime work must
run on device; the network-disabled Android round-trip gate is now explicit in R01.


The owner selected MishMash's microvoxel landscape reference and requested an
[autonomous swarm development roadmap](AUTONOMOUS_MICROVOXEL_ROADMAP.md).
Use it as the outcome-level follow-up to the detailed completion plan: extensive
supervised agent research, implementation, testing and review; technical capability
and functional Android acceptance first; measurement/optimization afterwards.
Continue accepted work and the existing lead's assignments rather than restarting
or creating a competing dispatch board. Routine PR completion is a continuation
point, not a request for further owner permission.

Use the [engine completion orchestration plan](ENGINE_COMPLETION_PLAN.md) for the
next dispatch order and M2–M6 closure gates. Reconcile historical ownership before
starting workers. Preserve the existing device-playable Voxel Relay sample; repair
the remaining engine acceptance gates.
PR #21 is merged; the reconciled starting point is `main = origin/main = 01ab450`,
with a clean checkout, no open PRs and no live Pi workers before this planning wave.
E0 and E1 are closed: audio passes 3 × 10 physical cycles; detail cadence passes
30 pinned suites without failures. Audibility and E7 integration remain open.
The owner confirms sufficient battery and prioritizes technical development and
functional Android acceptance now. E2 overhead/noise and M5 thermal/performance
optimization follow technical completion; they do not block implementation.
Deferred measurement gates remain open, not passed.

Continue the engine-completion campaign in [PERFORMANCE_PLAN.md](PERFORMANCE_PLAN.md)
and its [task list](PERFORMANCE_TASKS.md): advance renderer, physics, streaming,
lighting and measured efficiency under the owner's 2026-09-08 direction to finish the
engine and focus on actual engine systems. Open capabilities include the
equivalent-quality ray/mesh/hybrid comparison and primary-path selection, full
Lumen-like indirect lighting/reflections, sustained efficiency and thermal
acceptance, streaming completion and the remaining M6 shared-service gates; [STATUS.md](STATUS.md)
records which gates remain.

The owner's latest routing instruction supersedes the historical six-family roster:
use supervised DeepSeek V4.1 Flash workers for implementation and planning, extensive
bounded Gemini initial reviews, and Opus only for a concrete escalation. Never
spawn or resume Astra as a worker, reviewer, consultant or delegated lead. The
existing lead owns integration and acceptance. Limit concurrency to three workers;
use independent review scopes and verify candidate findings before acting on them.
The updated completion plan supplies the next dispatch contracts; E3 fixtures,
E4 correctness and E7 reuse work can proceed while device measurements are blocked.

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
