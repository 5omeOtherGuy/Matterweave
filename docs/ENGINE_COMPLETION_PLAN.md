# Engine completion orchestration plan

Written 2026-09-12 against the repository's recorded evidence. This is a delivery
plan, not a report of newly executed engine checks or a calendar promise.
[STATUS](STATUS.md) owns accomplishments; [REQUIREMENTS](REQUIREMENTS.md) and
[ROADMAP](ROADMAP.md) own acceptance. This document supplies the next execution
order for M2–M6. It supersedes the older showcase-first task ordering where that
conflicts with engine completion; retain the measurement and review contracts in
[PERFORMANCE_PLAN](PERFORMANCE_PLAN.md) and [PERFORMANCE_TASKS](PERFORMANCE_TASKS.md).

## Finish line

Finish the native Android engine demonstrator defined by M0–M6: reusable Rust
systems, meaningful editable fine voxels, stable automatic detail, dynamic indirect
lighting and reflections, complex physical interaction, bounded streaming and
measured sustained operation, proven by two playable samples sharing the engine.
The dense wetland remains a mandatory validation workload.

Every required gate must pass on its declared scope before calling the engine
complete. An unavailable test remains open; it can permit an explicitly incomplete
checkpoint, but cannot count as acceptance. Optional research may be deferred with
a reason. Commercial production readiness, an editor ecosystem, multiplayer and
store publication are outside this finish line.

Use the OnePlus 13 as the existing reference, with an explicitly device-specific
validated profile. API/ARM64/Vulkan compatibility claims remain separate from
physical-device evidence. Broader device claims require additional hardware results;
do not imply that one flagship validates the provisional Android floor.

## Starting position

- Preserve the native foundation, voxel core, Rapier adapter, automatic detail,
  background preparation, shadow reuse and production frame pacing already built.
- v0.5.0 is the latest prerelease recorded in the repository. Later merged and
  integration-branch changes require source reconciliation before new dispatch.
- The bounded renderer comparisons establish feasibility, not equivalent production
  quality or a primary mobile path. M2 remains open.
- The 2026-09-12 reflection gates pass on the reference phone, with a recorded
  additional 7.45 ms GPU cost per frame in that diagnostic. This does not establish
  full GI/reflections quality, wetland cost or sustained efficiency.
- Voxel Relay was solved end to end on that phone. M6 now needs reusable-service,
  persistence, authoring and release acceptance, rather than another second game.
- Audio's physical diagnostic fails its suspend assertion in four of four runs.
  Diagnose the backend/control/observation contract before selecting a fix.
- Collision publication, streaming stress, detail transitions, lighting quality,
  instrumentation overhead and sustained thermal acceptance retain open gates.

Historical branch/phone assignments and some narrative summaries disagree with newer
evidence. Reconcile them in E0; never resurrect a worker or redo a shipped feature
solely because an old board entry says it is pending.

## Ownership and dispatch

One lead owns integration, shared interfaces, Android app wiring, the phone,
acceptance decisions, global documentation and delivery. Keep at most three active
workers alongside the lead. Use isolated worktrees with nonoverlapping owned paths;
serialize phone access, GPU captures and large Android builds. Benchmark data and
worker execution artifacts belong under `/mnt/bench`, with compact durable records
and artifact hashes in Git. Remove generated temporary-worktree build targets after
their processes stop; preserve shared caches.

Use the existing [board](performance/board.json) as the sole live assignment
authority. The task IDs below are planned work packages, not a second live board.
Archive historical attempts before updating their ownership; retain partial output
and reject stale handoffs using attempt ID, base revision and interface version.

Preserve the campaign's six-family participation: Astra lead plus bounded Opus,
Muse, GLM, Gemini and Hy4 implementation or review tasks. Tentative routing is Opus
for subsystem changes, GLM for bounded numeric/resource work, Hy4 for independent
fixtures or fault cases, and Muse/Gemini for independent reviews. These are starting
assignments, not capability rankings. Verify available routes, task qualification
and authorized spending limits before dispatch; record unavailable routes and
reroute useful work without purchasing credits or inventing participation.

For each candidate, freeze the source, obtain independent Muse and Gemini reviews
without sharing their findings, then perform the lead's own review and reproductions.
If a route is unavailable, record it and use an available independent reviewer while
retaining the unmet route outcome. Reviewer agreement is not evidence of correctness.
With three worker slots, review waves replace implementation slots rather than
silently exceeding concurrency. Re-review corrective changes and affected seams.

Every worker brief must contain:

- One observable outcome, exclusions, exact base, owned paths and dependencies.
- Versioned input/output contracts, fixture facts and resource limits.
- A discriminating definition of done; worker checks and lead/device checks separated.
- Stop condition, bounded attempt budget and partial-handoff instructions.
- A unique engineering log, artifact fingerprint and PASS/FAIL/NOT RUN results.

## Work packages and exit gates

| ID | Deliverable and ownership | Dependencies | Exit evidence |
| --- | --- | --- | --- |
| E0 | Lead reconciles checkout, main, open PRs, active processes, board and evidence; creates the current requirement-to-gate ledger. | None | Exact baseline commit/APK and toolchain; existing work preserved; every M2–M6 gate has an owner, dependency, evidence level and next command; phone and build ownership explicit. |
| E1 | Audio worker diagnoses and repairs suspend/resume; physics-test worker repairs the remaining detail-cadence observation race. Lead integrates separately. | E0 | Audio regression distinguishes callback-thread progress from control-thread state, then passes repeated physical suspend/resume and shutdown checks; collision test covers immediate and delayed completion without depending on worker slowness. No production invariant weakened to pass a test. |
| E2 | Measurement worker closes instrumentation and fixture qualification; lead owns mobile repeatability. | E0 | CPU busy/wall/waits and GPU/presentation identities remain distinct; capture overhead and same-build noise measured on phone; missing samples rejected or quantified; fixed quality, replay and memory accounting frozen. |
| E3 | Renderer worker compares ray, mesh and hybrid at equivalent quality; detail worker supplies shared representation/quality fixtures. | E0; E2 before performance selection | Close detail, thin foliage, occlusion, edits, moving bodies and orthographic fixtures compared with total preparation/upload/render costs and matched motion captures. Primary path selected with evidence; ADR-0005/0006/0007 resolved to the extent their gates pass. Retain references and useful failed experiments. |
| E4 | Core and physics workers close bounded streaming, collision publication and destruction contracts; lead wires production paths. | E0; E2 for cost acceptance | Reversal, eviction, edits, cancellation and occupied additions cannot publish stale state or remove support. Declare publication/pending behavior, latency and queue/staging/residency limits. Exercise 64-piece destruction, constraints, mass/inertia, persistence and 20 lifecycle/reset/load cycles; run graphics-integrated Android stress. Resolve M3 and ADR-0010. |
| E5 | Lighting and detail workers finish dynamic GI/reflections and visually stable detail using the selected renderer. | E3; E4 publication contracts; E2 measurements | Moving lights/sun and open/closed enclosures demonstrate indirect response, color bleeding and bounded leakage; scene edits and moving reflectors update correctly. Measure lighting latency, upload cost and total frame cost. Approach/retreat/zoom captures pass thin-feature, seam and silhouette criteria. Evaluate reconstruction against native resolution, including ghosting/disocclusion. Resolve M4 and ADR-0008/0009. |
| E6 | Lead runs mobile acceptance; workers address one measured bottleneck per candidate and inspect justified accelerator opportunities. | E2–E5; begin profiling earlier | Matched fixed-quality comparisons followed by sustained finalist runs cover stationary, travel, destruction and worst wetland vistas. Explicit memory bounds, frame distributions, thermal behavior and supported profiles documented. CPU/GPU/accelerator inventory has measured adopt/defer decisions; energy remains unknown without qualified telemetry. Resolve M5. |
| E7 | Framework worker audits and completes shared services used by wetland and Voxel Relay; lead owns Android integration. | E0 audit; E1 and E3–E5 before final acceptance | Both samples build and run from the same engine without forks; seeded behavior and persistence pass; mobile input, lifecycle, minimal audio/animation/UI/assets and extension steps are verified. Physical simultaneous touch and lock/unlock are tested explicitly. Resolve M6 and ADR-0011/0016 where justified. |
| E8 | Lead freezes, verifies and delivers the finished demonstrator. | E1–E7 accepted | All required gates and critical findings closed; clean-checkout builds, final CI and physical sample acceptance pass for the exact candidate; merged PRs and testable development APK with manifests, hashes, captures, reports and limitations. No store publication or owner license selection. |

E5 begins with bounded experiments while E3 runs, but shared renderer changes wait
for interface decisions. E7 begins as a reuse audit while core work proceeds;
sample polish cannot displace E3–E6. E6 measures intermediate slices immediately,
but final sustained acceptance must include the complete integrated quality profile.

## First dispatch and subsequent waves

1. **Reconcile and unblock:** lead executes E0. Then dispatch three bounded tasks:
   E1 audio diagnosis, E2 instrumentation-gap closure, and E4 collision/streaming
   contract audit plus the pending-window regression. Lead checks integration state
   and prepares exclusively owned phone fixtures. Separate overlapping physics
   edits within the third task rather than launching a fourth writer.
2. **Select and bound:** after the first review/integration wave, dispatch E3 renderer
   comparison, E4 core streaming closure and E4 physics stress into separate paths.
   Lead serializes app wiring and device experiments. Freeze common fixture and
   representation interfaces before dependent writers start.
3. **Complete fidelity and reuse:** dispatch E5 lighting, E5 detail transitions and
   E7 framework completion. Keep lighting/render backend edits separate from detail
   source/selection changes; lead owns any shared renderer entry points.
4. **Prove and deliver:** run E6 matched/sustained acceptance and E8 release closure.
   Use workers for reproduced bottlenecks and independent review while the lead
   holds the phone. Repeat only the gates affected by changes, then perform the
   final integrated acceptance once the candidate is frozen.

The critical path is E0 → E2 → E3 → E5 → E6 → E8. E4 publication semantics feed
E5; E1 and E7 must finish before E8. Reassess the critical path after each accepted
wave. Estimate durations only after E0 and the first measured work packages reveal
their scope; no credible completion date follows from the present feature list.

## Measurement and decision rules

Apply [BENCHMARKS](BENCHMARKS.md) and the campaign measurement contract. Freeze
resolution, quality, scene, seed, replay, build and refresh policy. Use genuinely
unplugged, cooled, matched conditions. Start with three alternating short A/B pairs;
finalists require at least 20 minutes after documented warmup (campaign: 120 s),
with repeat runs where needed to distinguish the claimed effect from variation.

The existing responsive target is p95 ≤16.67 ms and p99 ≤25 ms; a 30 Hz fidelity
profile is separately labeled. These are working targets, not new owner mandates
or achieved results. Predeclare quality, latency and memory thresholds before each
experiment. An unmet target stays open or receives an explicit evidence-backed
engineering revision preserving the original result; do not reduce quality silently.

Prefer one causal change per comparison. Apply the existing improvement/no-regression
rule (improvement above both 5% and same-build variation; guard repeatable regressions
above 5% outside noise). Report intentional tradeoffs separately. After two
inconclusive iterations on one bottleneck, improve attribution or choose another
measured cost. A faster frame with higher heating is not a blanket efficiency win.

## Integration, evidence and release closure

For each accepted slice, run relevant checks from [DEVELOPMENT](DEVELOPMENT.md),
including focused regressions, workspace tests, formatting, strict Clippy, Android
build/package checks and the affected real-device gates. Every ignored test must
be classified: an outstanding required gate cannot disappear behind an ignored count.
Run `python3 tools/check_docs.py` for documentation changes. Inspect the final diff.

Record exact source, device/OS/driver, fixture hashes, conditions, commands and
PASS/FAIL/NOT RUN results. Store large evidence in durable workflow/release artifacts
with checksums; local `/mnt/bench` paths alone are not a final evidence handoff.
Commit, push, open PRs, resolve actionable review/CI findings and merge after required
checks under the existing delivery authorization. Preserve protections and history.

At every wave update STATUS, affected ADRs, ROADMAP, the board and engineering logs.
Each open gate has a next runnable task; each accepted claim links to evidence.
If phone access, provider access or a required review is unavailable, retain the
checkpoint, record the exact blocker and continue independent work. Ask the owner
only for a missing owner decision or inaccessible capability that blocks necessary
progress. Licensing, purchases, access changes and production distribution remain
owner-controlled.

The final handoff must let a fresh session build the engine, run both samples,
reproduce acceptance and understand every supported limit using this repository
and its durable artifacts alone.
