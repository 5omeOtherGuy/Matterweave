# Performance optimization campaign

Baseline release v0.3.0, source `2ce6ce3157e791ab6dd49ed00cd865ee5c1cacc6`.
Current state and remaining gates: [STATUS](STATUS.md). Task definitions and their
done criteria: [P00–P07](PERFORMANCE_TASKS.md). Measurement protocol:
[BENCHMARKS](BENCHMARKS.md). Showcase specification: [SHOWCASE](SHOWCASE.md).

## What is optimized

Two required outputs:

- Measured overall engine optimization, demonstrated on the OnePlus 13, with
  comparisons showing where performance improved, regressed or stayed inconclusive.
- The explorable [showcase](SHOWCASE.md): high voxel count, dense vegetation,
  recognizable modeled mushrooms, water and complex terrain, from original assets.

The showcase is mandatory, not an optional future demo. Do not call the campaign
complete with only an optimized empty scene, instrumentation, a backlog or a host
preview.

Correct edits, queries, terrain/body collisions, save compatibility and lifecycle
recovery must hold throughout. Visibility, LOD and idle policies must not silently
change game rules. The engine remains reusable Rust with ash/Vulkan; Rapier is the
current physics baseline. No language/backend rewrite or new physics package is
justified by this optimization request alone. Prototype findings are not product
guarantees.

Evidence: versioned benchmark scenes and replays; compact reports and raw evidence
with hashes; per-worker engineering logs; the model results ledger; the lead
execution log; accepted/rejected experiment decisions; reviewed and merged PRs; and
a tested GitHub APK with its build manifest.

## Loop

1. **Freeze.** Record the iteration ID, baseline/candidate source, scene
   configuration hashes, one principal hypothesis, primary metric, correctness and
   quality guards, allowed files, resource limits and stopping rule. Settle
   candidate-dependent interface decisions before dependent writers start.
2. **Measure.** Choose a discriminating measurement from a source hypothesis;
   source inspection alone does not establish a bottleneck. Separate CPU busy time,
   elapsed stages, GPU execution intervals, synchronization waits and presentation.
3. **Implement.** One causal optimization per candidate. Integrate interacting
   changes serially so individual and combined effects remain attributable.
4. **Worker handoff.** Require artifact fingerprint, relevant diff,
   definition-of-done results and engineering log. Verify the runner and children
   are stopped. Freeze the candidate before reviewers begin; workers do not modify
   reviewed files.
5. **Independent review.** Reviewers receive read-only frozen sources and work
   without each other's findings. Every finding needs file/line, a concrete trigger,
   consequence, supporting evidence, uncertainty and a proposed discriminating
   check. "No findings" is a valid result; a reviewer vote is not acceptance.
6. **Lead review.** Check the changed code, invariants and integration seams
   independently, verify candidate findings, and resolve disagreements with
   reproductions or explicit evidence.
7. **Verify.** Run relevant host tests and validation, then matched phone
   measurements and visual/interaction checks. A host win does not qualify a mobile
   optimization.
8. **Decide.** Accept, reject, revise or mark inconclusive with evidence. A
   corrective patch changes the candidate fingerprint and needs review of the
   correction and affected invariants.
9. **Advance.** Merge accepted work after the required checks; update the baseline,
   the results ledger and the docs; publish useful tested checkpoints. Repeat from
   the highest remaining measured cost or unmet showcase requirement.

## Execution

The campaign ran with multiple models (Opus, Muse, GLM, Hy4, Gemini and the Astra
lead). Per-route outcomes and limits are the [model results ledger](performance/models.md).
Raw execution logs are under [performance/logs/](performance/logs/), including the
[completion log](performance/logs/completion-execution.md) and the
[lead native integration log](performance/logs/lead-native-p02.md). A failed or
unavailable route is an honestly recorded result, not a delivery blocker.

Do not buy subscriptions, replenish paid credits, bypass limits, change credentials
or publish a store release.

## Measurement contract

Use [BENCHMARKS](BENCHMARKS.md), with these campaign requirements:

- Preserve the v0.3 raw baseline as history, but collect a new matched reference.
  Its USB-powered, already-hot, severe-throttling run cannot be the causal comparator.
- Use wireless debugging on a trusted network where available and physically
  disconnect charging. Verify the actual USB/AC/wireless power state. Do not treat a
  simulated battery-service state as disabled charging or invent charge-control paths.
- Freeze brightness, output/internal resolution, refresh policy, quality, seed,
  camera/input replay, body/vegetation counts, build flags, profiling mode and OS mode.
  Record case and ambient conditions. Start matched pairs with the same thermal status
  (prefer no throttling), battery temperature within 1°C and skin within 2°C when
  available; require stable readings for two minutes. These are initial comparison
  tolerances, not evidence of equal electrical power. Report exceptions and
  inconclusive pairs.
- Measure CPU busy time with available thread/process clocks or a qualified trace,
  separately from wall time. Mark acquire/fence/present waits explicitly. Add work
  counters for physics active/sleeping bodies, mesh generation/uploads, shadow passes,
  saves, allocations/retained capacity, preparation queues and collision publication.
  Positive stage duration alone does not mean useful work occurred.
- Keep redraw, submitted-frame, GPU-completed-frame and actual-presentation identities
  distinct. Retry draws must remain visible. A present API call is not a scanout
  timestamp. Do not fabricate a CPU/GPU/compositor join where IDs or clocks lack a
  mapping. Qualify new timestamp boundaries and measure instrumentation overhead on
  the phone.
- Report p50/p95/p99, >33.3/>50/>100 ms intervals, missing-sample coverage, startup
  and edit-to-visible/collision latency, process/graphics memory and queue/retirement
  peaks. Track temperature/throttle transitions. Battery percent alone is not energy;
  use calibrated available energy/current telemetry or mark energy unknown.
- Use short matched A/B screens first (initially 3 alternating pairs of 60–120
  seconds, after documented warmup/cooldown). Compare run-level results and observed
  noise; thousands of correlated frames are not thousands of independent experimental
  runs. Only finalists require 20-minute sustained captures after 120 s warmup, with
  matching reference conditions and repeats where comparative claims need them.
- Fixed-quality work reduction, frame-cap/idle scheduling, quality adaptation and
  alternative build profiles are separate experiments. Never hide reduced resolution,
  vegetation, voxel detail or simulation behind an "optimization" label.

## Acceptance

Initial acceptance rule: correctness and visual gates pass, primary improvement
exceeds both 5% and the measured same-build variation for that metric, and no guarded
metric shows a repeatable >5% regression outside noise. This is a working decision
rule, not a promised win or statistical confidence claim. Explicit tradeoffs require
separate reporting and a justified acceptance decision; otherwise reject or defer.
Calibrate sensible tolerances for memory, latency and images before the experiment,
not after seeing results. Do not optimize the smoothed HUD counter.

Campaign completion requires all showcase gates, matched phone evidence for accepted
optimizations, no unresolved release-blocking correctness/lifetime defects,
all-model qualification outcomes, passed relevant CI and a published testable APK.
Aim for a 60 Hz responsive profile (p95<=16.67 ms, p99<=25 ms) on a declared fixed
configuration; this is an engineering target, not an achieved result. If it is not
met, publish the actual result and remaining bottleneck and do not claim the
performance target complete. An explicit 30 Hz fidelity profile can be evaluated
separately; it must not silently replace the responsive target. No universal
flagship or device claim follows from one phone.

## Workloads and stopping rules

Always retain a small diagnostic scene, but add increasing portions of the showcase
from the first content iteration. Mandatory workloads: cold launch/load; settled
stationary view; slow look/walk; fast travel and reversal; 64-body fracture/settling;
terrain edits and saves; showcase water/vegetation vistas; foreground/background and
resume/recreation; bounded memory-pressure and repeated reset/load cycles.

Execution order is [P00–P07](PERFORMANCE_TASKS.md): trustworthy instrumentation and
qualification; idle/dynamic work reduction; representative showcase tile and fine
voxel representation; full dense showcase; active streaming/render/memory work;
water/material polish; final combined regression and release. Independent asset and
test lanes can overlap engine work once interfaces are fixed. Do not defer all art
until optimization ends; conversely, do not build millions of detail cells using an
unbounded prototype layout before resource limits are checked.

Stop an individual experiment after its defined failure, deadline, qualifying win or
one informative follow-up. Reject fixes that only relocate waits or reduce work by
breaking responsiveness, simulation, geometry or quality. After two inconclusive
iterations on the same bottleneck, return to attribution or choose another measured
cost; do not keep speculative rewrites alive. Pause dispatch on quota/provider failure
and retain artifacts. End a work block with a usable checkpoint if external conditions
require it, explicitly listing outstanding deliverables.

## Coordination and logging

Keep at most three active workers plus the lead, and serialize phone access, GPU
captures and large Android builds. Retain v0.3's coordination bounds: scoped startup
view <=6 KiB, status <=2 KiB, messages <=1 KiB, inbox <=4 messages/8 KiB. These bound
coordination messages, not useful long source material; point to artifacts instead of
replaying logs. Keep one canonical live assignment authority
([board.json](performance/board.json)); never operate two conflicting live boards.
Each assignment records task/attempt/owner/profile, exact baseline, unique workspace,
owned paths, interface versions and dependencies, run/session path, artifact/log path,
state and acceptance evidence. States: running, submitted, review-needed, accepted,
rejected, blocked. The lead owns global docs, manifests, integration and releases.
Update STATUS with the exact shipped work, next runnable task, unresolved gates and
device ownership.

Each implementation brief includes outcome/exclusions, verified fixture facts, owned
files/interfaces, a measurable definition of done, worker-vs-lead checks, limits,
stop condition and a unique engineering-log path. Logs record Actions Taken, Issues &
Friction, Decisions & Rationale, Solutions Applied and Insights at major steps.
Handoffs include pass/fail/not-run evidence and a concise log summary. Keep the lead
execution log and accepted experiment/model summaries in Git; retain larger captures
and worker sessions under `/mnt/bench` with appropriate release artifacts and hashes,
and never publish raw provider sessions. Release manifests identify source, build
flags, test conditions, signing identity and APK checksum. Preserve user saves and
restore the user's scene after tests.
