# Autonomous microvoxel roadmap

Owner-requested follow-on roadmap (2026-09-12) for the autonomous project: reach the
microvoxel landscape and interaction quality bar on native Android through extensive
agent-swarm orchestration. It complements [ENGINE_COMPLETION_PLAN](ENGINE_COMPLETION_PLAN.md);
it does not replace the M0–M6 acceptance and may overlap or reorder existing Phase A
work where evidence justifies it. [STATUS](STATUS.md), [ROADMAP](ROADMAP.md) and the
[board](performance/board.json) remain the live authorities; the lead updates those
entry points, workers do not.

The owner transferred ongoing development, orchestration and Android integration to
the current Codex lead on 2026-09-12. The other session finishes its existing merge
workflow. Preserve its active destruction/lighting deliveries, reconcile live Pi
workers before dispatch, and continue from the shared board without duplicate work.

## Goal (north star)

Reference: "Solving the hardest problem in my Micro Voxel Engine" —
<https://www.youtube.com/watch?v=xGkWWfO87no>. The title was verified via oEmbed;
playback was blocked in this environment and the thumbnail was inspected, so no
full-video-watched claim is made. The creator's first-party writeups confirm the
direction: meshing and multiresolution macrochunks with persistent prop edits
(<https://www.reddit.com/r/GraphicsProgramming/comments/1vrqmul/improving_render_distance_in_my_micro_voxel_engine/>)
and an earlier 10 cm voxel engine
(<https://www.reddit.com/r/GraphicsProgramming/comments/1udkyzj/i_built_a_microvoxel_engine/>).

The owner additionally requires the entire runtime on device, without required cloud
rendering, simulation or world storage. [Virtual Matter and Lay of the Land](performance/virtual-matter-and-lay-of-the-land.md)
are assessed for transferable concepts and reuse fit; neither is an adopted dependency.
The S5 technical gate includes network-disabled cold start, generation/load, play,
edit, save, process restart and reload on Android (not yet run).

Target outcome on native Android:

- detailed microvoxel landscapes with fine geometry nearby and rich vegetation;
- coherent distant terrain and stable transitions between detail levels;
- editable, persistent world state plus integrated lighting and interaction.

Explicit non-claims and boundaries: no exact desktop scale or frame-rate requirement is
imported; no mandatory smooth-SDF representation and no ray-only rewrite (the ADR-0006
comparison stays open); simulations shown in unrelated videos are not scope unless the
owner or a recorded decision adds them.

## Stages, outcomes and exit gates

The simplest sequence reaching the goal; stages may run in parallel or reorder when
evidence justifies it, and completed work is never redone.

| Stage | Outcome | Exit gate (functional unless stated) | Milestones |
| --- | --- | --- | --- |
| S0 Consolidate | One reconciled ledger of existing capability, open gates and owned paths; no duplicate or stale assignments | Source, board and live-worker state agree at a frozen base SHA; unrun gates listed explicitly | M0/M1 preserved; current M2–M4/M6 mapped |
| S1 Close-up quality | Representative microvoxel close-up on device: fine geometry, vegetation, materials, edits | Android capture and functional checklist against declared quality criteria; no fixed-resolution mandate | M2 |
| S2 Landscape scale | Coherent distant terrain via bounded streaming/LOD; edits persist and stale derivations never publish | Streaming/edit round-trip and transition checks on device; declared residency/queue bounds | M2/M3 |
| S3 Unified lighting | Detail and dynamic objects receive integrated indirect light and reflections with stable temporal response | Functional probes for moving geometry, lights and enclosures plus declared response/latency criteria | M4 |
| S4 Shared-engine integration | Both samples playable on one engine: input, lifecycle, persistence, audible output, physical interaction | Two-sample Android run without engine forks; required human gates explicitly passed for technical acceptance; otherwise remain open | M6 with M2–M4 |
| S5 Technical freeze | Frozen, reproducible technical candidate before any measured claim | Clean-checkout build, APK identity, workspace tests, fmt/Clippy, `check_docs.py`, reviewed merged PRs | E8-T equivalent |
| S6 Performance/thermal | Measured efficiency only after the freeze | E2 qualification, equivalent-quality comparison and M5 sustained/thermal on the frozen profile | M5 (last); closes with E8-F |

S0–S5 are engineering phases; S6 is deliberately last. M3 streams into S2, M4 feeds S3,
M6 feeds S4, and M5 is measured, not inferred. Nothing in S0–S5 makes a measured-fastest
or thermal claim. Exit gates are verifiable outcomes, not calendar promises.

## Autonomous swarm charter

**Orchestration loop.** The orchestrator continuously reconciles repository source, the
board and live worker state, selects ready work, and delegates extensively through
supervised Pi workers across waves. Every active stage uses multiple bounded agents
for research, code, independent testing and review as useful; delegation is the default
execution method, not an optional final review. The loop is: short research feeding a decision → prototype →
implement → independent tests and reviews → Android integration by the lead → passing,
reviewed, merged PR → next stage, automatically. Research is time-boxed and must feed a
decision; no endless papers and no duplicate reviews.

**Concurrency and delegation.** The active concurrency cap today is three workers plus
the lead; the total swarm across successive waves is not limited to three, because work is split
into bounded independent tasks. One shared board owner and one device lease exist; the
board is the sole live assignment authority. Optional nested delegation is allowed only
when expressly assigned with a shared budget and reported back to the board.

**Model routing and prohibitions.** DeepSeek V4.1 Flash (`deepseek-flash-go`, effort
`max`) is the primary research, implementation, testing and planning worker. Gemini runs numerous focused
initial and change-specific reviews. Opus is a justified escalation only, with a
recorded blocker and prior attempts. Astra must never be delegated any role, alias or
consultancy.

**Handoffs and acceptance.** Worker completion is not acceptance: the lead verifies
against the exit gate and updates STATUS/ROADMAP/board. Handoffs are persisted in the
repository and performance logs so a fresh session can resume. No unattended execution
is promised without a live host/session, and no new scheduler or background service is
requested.

**Stops and gates.** Continue independent tasks; stop only for genuine owner, access or
irreversible blockers. No routine approvals and no stopping after each PR — the normal
delivery workflow (commit, push, PR, review, merge) proceeds under existing authority.
Respect quota and deadlines; stop unproductive tasks. Hearing and physical touch remain
human-only gates and stay explicit.

## Reference documents

- [ENGINE_COMPLETION_PLAN](ENGINE_COMPLETION_PLAN.md) — Phase A/B packages and slices.
- [REQUIREMENTS](REQUIREMENTS.md), [ROADMAP](ROADMAP.md), [STATUS](STATUS.md).
- [BENCHMARKS](BENCHMARKS.md) and [measurement qualification](performance/measurement-qualification.md) for S6.
- [COMPONENT_SELECTION](COMPONENT_SELECTION.md) and ADR-0014/ADR-0015 for reuse rules.
- Delivery log: [microvoxel-roadmap-deepseek](performance/logs/microvoxel-roadmap-deepseek.md).
