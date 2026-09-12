# Engine completion orchestration plan (v2, owner-reprioritized)

Revised 2026-09-12 at `01ab450` under the owner's priority change: finish engine
capability now; measurement and thermal optimization later. [STATUS](STATUS.md) owns
dated evidence; [REQUIREMENTS](REQUIREMENTS.md) and [ROADMAP](ROADMAP.md) own
acceptance. This supersedes the v1 E0–E8 order where they differ; v1 is replaced, not
passed. This is a delivery plan, not a report of newly executed checks or a calendar
promise.

## Owner priority change (2026-09-12)

- **Phase A — technical completion now:** M2 representation/renderer production
  integration, M3 streaming/physics, M4 GI/reflections/detail and M6 shared services,
  with Android functional tests run as each slice integrates. A provisional,
  evidence-backed engineering path is allowed; no "measured fastest" claim and no final
  performance selection are made in Phase A.
- **Phase A quality bar:** functional fixtures plus explicit, observable and declared
  quality criteria are sufficient now. No fixed-resolution (e.g. 1080p) mandate and no
  invented 1% ghosting threshold. Cost ranking and reconstruction comparisons are
  Phase B.
- **Phase B — after technical completion:** E2 capture overhead/same-build noise,
  equal-quality cost ranking and final primary-path selection, M5 sustained/thermal/
  optimization and the full measured acceptance. **M5 is deferred by the owner, not
  passed.**
- **E2 is not a prerequisite** for Phase A implementation or functional acceptance.
- No battery/charge blocking and no mandatory thermal campaign in Phase A; the owner
  reports enough charge for functional work, and Phase A never gates on charge. Each
  Phase B matched pair still requires fresh qualified thermal conditions.
- **Technical completion is distinct from the original M0–M6 measured acceptance.**
  Phase A closes the technical demonstrator checkpoint E8-T. The original full-scope
  measured acceptance closes later as E8-F. Both stay visibly open until evidenced.

## Reconciliation baseline (E0/E1 closed)

- PR #21 merged at `01ab450`; `main` = `origin/main` = `01ab450`, clean at
  reconciliation; no open PRs and no live workers. E0 is closed.
- E1 audio closed: real-AAudio diagnostic PASS, 3 of 3 runs × 10 pause/recreation
  cycles at `a612f20`, 0 xruns and 0 device errors. Human audibility is still
  unverified and remains a distinct open gate.
- E1 detail-cadence race closed: 0 failures in 30 pinned 11-test suite runs on the
  merged PR #21 correction. `detail_cadence.rs` is frozen regression; no edits without
  lead approval.
- Reflections: short functional device gates pass; +7.45 ms GPU per frame when enabled
  is the recorded functional cost. The 7.974 ms single fence block is **one
  observation with no established cause** — an outlier hypothesis to check, not a
  known regression.
- P02's thermal regression (skin ≈49 °C, thermal status 2 vs reference ≈39 °C/status 0)
  stays open until retested on the final equivalent profile in Phase B. Historical P02
  pairs are throughput/thermal tradeoffs, not efficiency wins.
- Open human/device functional gates, explicit: hearing confirmation, physical
  simultaneous touch, physical lock/unlock, keyguard unlock before any graphics run.
- The repo holds many stale worktrees. They are not live ownership; archive or ignore
  them, never resurrect a worker or redo a shipped feature because an old entry says
  pending.

## Routing, authority and anti-duplication

- **DeepSeek V4.1 Flash through Pi** is the primary implementation and planning
  worker for Phase A packages and briefs.
- **Gemini** performs extensive initial **read-only** reviews in bounded waves across
  six distinct scopes (G1–G6 below). Reviews return findings; reviewers do not edit the
  repository and get no device access.
- **Opus** is used only as an evidenced escalation: a repeated unexplained failure
  whose blocker and prior attempts are recorded before dispatch. An escalation is an
  engineering response, not a capability ranking.
- **Never use Astra workers, reviewers or consultants under any alias.**
- **The lead owns integration, shared interfaces, app wiring, the phone, acceptance
  decisions and final verification.** One lead, at most three concurrent workers total
  alongside the lead (implementers plus reviewers together). A review wave replaces
  implementation slots rather than exceeding the cap.
- Model participation or agreement is not a score and never a correctness claim. All
  claims need discriminating evidence.
- **`docs/performance/board.json` remains the sole live assignment authority.** The
  package tables and slices below are planned only; the lead writes each task and
  attempt into the board before dispatch and updates it at every accepted wave. No two
  owners share a path: workers own the paths listed in their slice, the lead owns
  `apps/explorer/src/**` app wiring, `crates/matterweave-render/src/lib.rs`,
  `docs/STATUS.md`, `ROADMAP.md`, ADRs, the board and the device.

## Finish line, scope and transparency

Phase A finishes the native Android engine demonstrator's **capabilities**: reusable
Rust systems, meaningful editable fine voxels, stable automatic detail, dynamic
indirect lighting and reflections, complex physical interaction, bounded streaming,
shared services proved by two playable samples, plus Android functional acceptance.
Phase B then reruns the complete original acceptance: E2 qualification, equivalent-
quality comparison and primary-path selection, M5 sustained/thermal/efficiency and the
measured limits table.

Scope remains one reference phone (OnePlus 13, device-specific validated profile).
Broader device claims require additional hardware. No commercial production-readiness,
UE5-parity, energy or thermal claim is made. R16 advanced neural paths stay optional
research after the Phase B lighting baseline and are not invented into a requirement.
Owner-only decisions (license, purchases, visibility, release signing, store
publication) remain out of scope.

## Milestone, requirement and gate map

| Milestone | Requirements | Phase A package — technical gate | Phase B — original measured gate | Current evidence | Next executable step |
| --- | --- | --- | --- | --- | --- |
| M0 foundation | R01, R13, R14, R17 | M0M1-R: clean-checkout build, APK identity, native frame/input/lifecycle smoke, save/reload regression | Re-verified on the measured candidate in E8-F; no separate measured M0 gate | M0/M1 shipped since v0.1; pinned toolchain; PR CI | Run M0M1-R in Phase A |
| M1 voxel slice | R01, R02, R10, R12, R13, R14 | M0M1-R revalidation; no redo | Re-verified on the measured candidate in E8-F; no separate measured M1 gate | v0.2–v0.5 slices; 30 pinned collision suites | Preserve; revalidate only |
| M2 representation/renderer | R02, R05, R08, R12 | **D1** production integration of a provisional evidence-backed primary path; detail/foliage/orthographic functional coverage | E3-M equivalent-quality ray/mesh/hybrid comparison, cost ranking, primary-path selection, ADR-0005/6/7 resolution | ray/mesh/hybrid feasibility checks pass; equivalent quality and selection open | Dispatch D1.1 (first wave) |
| M3 physics/streaming | R06, R07, R12, R20 | **D2** bounded streaming/collision publication completion, destruction, persistence bounds, graphics-integrated 64-piece functional run in Phase A | Measured streaming/collision/destruction cost and bounds on the final candidate under E2 qualification; Jolt only on documented major advantage | headless streaming stress passes; graphics-integrated 64-piece acceptance open | Dispatch D2.1 (first wave) |
| M4 lighting/detail | R08, R09, R15 | **D3** production detail/moving-object lighting integration: indirect volume is unit-voxel only (`indirect.rs`) and reflections make dynamic/static/detail meshes nonreflective (`reflection.rs`); extend both to production geometry plus dynamic response and a lighting-latency bound; detail transitions coordinate with D1 | E5-M quality/latency thresholds and reconstruction comparison under E2 | short reflection functional gate passes (+7.45 ms); production geometry coverage and full GI open | Dispatch D3.1 when a slot frees |
| M5 efficiency | R03, R04, R18 | **Deferred by owner** | E6 matched sustained acceptance: profile/device/driver/resolution/distributions/memory/thermal | historical heating tradeoff; P02 thermal open | Phase B only; not passed |
| M6 shared services | R05, R06, R10, R18 | **D4** real shared-service integration starting immediately: `apps/explorer` has no `matterweave-audio` dependency today, so audio + lifecycle must be wired into both samples, plus UI/input/persistence reuse without forks; Phase A human/device gates: hearing confirmation, physical simultaneous touch, physical lock/unlock | Reuse/release acceptance evidence on the measured candidate at E8-F | Voxel Relay plays end to end; audio unwired; human gates open | Dispatch D4.1 immediately (does not wait for D1–D3) |
| Cross-cutting | R11, R19 | component-selection evidence inside every package | process/applied decisions with measured fit | ADR-0014/0015 accepted | Apply per package |
| Research/optional | R16 | none | bounded experiment if later justified | none claimed | Defer, no invented requirement |

R11/R19 apply inside each package's component decisions; R20 is exercised in D2/E4.
R16 is optional and never blocks Phase A or the measured closure.

## Phase A work packages

| ID | Deliverable | Depends on | Phone? | Exit evidence |
| --- | --- | --- | --- | --- |
| E0 | Reconciliation: baseline, PR/worker state, gate ledger | — | No | **Closed** at `01ab450` |
| E1 | Audio suspend/recovery and cadence regression | E0 | Done | **Closed**: `a612f20` audio 3×10; 0/30 pinned cadence suites (PR #21) |
| D1 (M2-A) | Production integration of a provisional primary renderer path; automatic-detail functional completion | E0/E1 frozen base | No (lead runs device) | Host functional fixtures pass; path flag documented; gap list; Android functional run of the integrated slice |
| D2 (M3-A) | Streaming/collision publication semantics completed; destruction/persistence functional completion; reproduce-or-dismiss the unverified candidate that conservative deferral starves publication while a body overlaps a prepared collider | E0/E1 frozen base | No (lead runs device) | Red-first tests; stale publication rejected; 64-piece host run; declared queue/staging/residency bounds; candidate reproduced or dismissed; graphics-integrated Android 64-piece functional run. Excludes merging stale results and an invented mandatory EngineContext |
| D3 (M4-A) | Production lighting coverage: detail meshes and dynamic mesh-only objects in indirect lighting and reflections, including moving objects; dynamic GI/reflection response and a lighting-latency bound; fence-outlier attribution deferred to Phase B | Frozen interfaces from D1/D2 where they meet | No (lead runs device) | Detail/moving-object probes pass functionally; moving sun/enclosure/reflector response recorded with declared observable criteria; latency observed; Android functional run |
| D4 (M6-A) | Real shared-service integration: add and wire the audio service into `apps/explorer` (no `matterweave-audio` dependency exists today), lifecycle/pause/recreation behavior, input/UI/persistence reuse, Voxel Relay + wetland without forks | None for service-side work; final sample integration after D1–D3 accepted interfaces | Lead wires + device | Both samples build/run with audio/lifecycle wired; host integration tests pass; Phase A hearing, physical simultaneous touch and lock/unlock human gates |
| M0M1-R | Preserve/revalidate shipped M0/M1 | E8-T candidate freeze | Yes | Clean-checkout build, APK identity, sample smoke, save/reload regression (Phase A) |
| E8-T | Technical demonstrator checkpoint | D1–D4 + reviews | Yes | Merged PRs, clean build, Android functional acceptance, docs updated |
| E2 | Capture overhead/same-build noise qualification | Phase B start | Yes | Accepted qualification contract; contract already accepted at `e436634`, phone numbers NOT RUN |
| E3-M | Equal-quality ray/mesh/hybrid comparison, cost ranking, primary-path selection | E2 + D1 | Yes | Matched comparison; ADR-0005/6/7 resolved to the extent gates pass |
| E5-M | Final lighting/detail quality and reconstruction/native cost comparison | E2 + D3 | Yes | M4 measured criteria and ADR-0008/0009 resolved for supported profile |
| E6/M5 | Sustained thermal/efficiency and optimization | E2, E3-M, E5-M | Yes | Original M5 report; P02 thermal retest on final profile |
| E8-F | Final measured delivery closure | E2, E3-M, E5-M, E6/M5 | Yes | Original M0–M6 evidence, limits table, final APK/CI/merged PRs |

Phase A device cadence (owner steering, 2026-09-12 after reboot): prioritize
coherent implementation batches and reduce repeated intermediate live checks.
Workers implement, host-test and repair reviewed changes; the lead runs Android
functional acceptance on the combined affected features before accepting their
behavioral gates. Reviewed host-only contract tests and opt-in diagnostic groundwork
may land with Android rows explicitly NOT RUN; that does not close a capability.
Immediate device investigation remains appropriate for Android-specific defects,
lifecycle behavior or uncertain GPU contracts. Workers never access the phone,
build the APK or capture device artifacts. E2 and cost/thermal work never block
Phase A. No prolonged harness-only detour before implementation.

## First dispatch — first vertical slices (planned; board owns dispatch)

`docs/performance/board.json` remains the **sole live assignment authority**: the lead
writes each task/attempt into it before dispatch; the tables below are planned only.
Every first slice is one PR-sized vertical outcome bounded by its **owned paths and
its definition of done**, at most one same-scope correction, then resplit or Opus
escalation with a recorded blocker. No slice carries a wall-clock time budget: a
deadline measures the machine and its current load, not the work, so it is not an
acceptance signal and never decides whether a slice is finished. Wave 1a runs D1.1, D2.1 and D4.1
(D4 does not wait for D1–D3); D3.1 enters when a slot frees after the lead freezes its
minimal detail/dynamic-object seam. At most three workers run alongside the lead;
Gemini reviews replace slots rather than exceeding them.

| Slice | One-PR outcome and owned paths | Interface/dependency | Worker DoD | Lead/device DoD and stop rule |
| --- | --- | --- | --- | --- |
| D1.1 (M2) | Connect the existing tested detail adapter to production wetland rendering. Own `crates/matterweave-detail/**` only if needed and a new `apps/explorer/src/detail_runtime.rs`; reuse `detail_check.rs` logic rather than reimplement LOD. | Lead owns wiring in `lib.rs`/`wetland.rs`, accepts source-version + camera + instance-batch contract before dispatch. | Production-facing adapter selects near/far detail, handles edited source revisions and invalidates derived batches; focused tests and fmt/Clippy pass. Source voxels and collision stay authoritative. | Lead wires, captures Android approach/retreat and edit behavior; one bounded correction, then resplit. |
| D2.1 (M3) | Reproduce-or-dismiss the unverified body-overlap starvation candidate with one discriminating host test and a publication-state note; apply the smallest correctness fix only if reproduced. Owns `crates/matterweave-core/tests/**`, `crates/matterweave-physics/tests/**` (excluding frozen `detail_cadence.rs`) and the minimal in-crate fix. | No public-surface change; queued → inflight → completed-unpublished → published states named in the note. **Excludes** merging stale results and an invented mandatory `EngineContext`. | The test discriminates a stall from correct deferral without assuming worker speed; no production invariant weakened; workspace tests, fmt, strict Clippy clean. | Lead reviews, then schedules the graphics-integrated Android 64-piece functional run; stop after one same-scope correction or at the public-surface boundary. |
| D3.1 (M4) | First production lighting slice: one moving mesh-only object or one detail volume receives correct indirect light and reflectivity in a functional probe; owns `crates/matterweave-render/src/{reflection.rs,indirect.rs,lighting.rs}` plus focused tests. Fence-outlier attribution is deferred to Phase B. | Starts only after the lead freezes the minimal detail/dynamic-object seam; no D1-owned writes; entry-point and shader registration diffs proposed to the lead. | Probe shows before/after response with an explicit observable criterion (response presence, thin-feature visibility, seam behavior); no fixed-resolution or percentage-ghosting threshold; tests, fmt, strict Clippy clean. | Lead integrates and runs the Android functional lighting slice; stop if D1-owned files overlap or after one same-scope correction. |
| D4.1 (M6) | Shared application audio adapter and two sample sound-event hooks. Own new `apps/explorer/src/audio_service.rs` plus focused tests; preserve existing verified audio crate unless a reproduced defect requires a separate fix. | Lead adds dependency/backend configuration and wires `experience.rs`, sample events and native lifecycle. Worker supplies an event enum and adapter methods using the existing service API; no Android types in engine gameplay APIs. | Host tests cover event/backpressure handling, mute/suspend/recovery and sample switch cleanup; backend selection is explicit. Both samples use the same adapter. | Lead verifies real Android backend and both-sample lifecycle, requests human audibility at functional acceptance; one bounded correction. |

A slice is bounded by scope, not by clock: **owned paths, the definition of done and
at most one same-scope correction**, then resplit or escalate (Opus only with a
recorded blocker). A worker stops when its outcome is met or it hits a stop condition.
Where a supervisor requires some deadline, set a non-binding safety ceiling to catch a
hung process and record it as such, never as a work budget. Source and fixture freeze: record base
SHA, fixture/generator hashes and the exact profile ID/default effort setting in the
log; do not rebase mid-slice; if `main` moves, the lead decides re-base or re-dispatch.
When a slice stops, the lead removes generated temporary-worktree build targets while
preserving shared caches. No worker touches another worker's paths, the board, STATUS,
ROADMAP, ADRs or the phone. Rejected or stale handoffs are archived with attempt ID and
base.

## Remaining implementation slices (dispatch after prerequisites)

| Sequence | Concrete outcome | Functional exit gate |
| --- | --- | --- |
| D1.2 → D1.3 | Complete production thin-feature/occlusion/orthographic/edit/moving-body fixtures, then seam/zoom/transition and residency fixes on the working renderer | Every scenario renders declared geometry correctly on Android; preserve ray/mesh/hybrid references and label production path provisional pending Phase B ranking. |
| D2.2 → D2.3 | Complete destruction/constraints/mass-inertia/persistence first; then integrated streaming reversal, eviction, cancellation and memory-pressure behavior | 64 pieces and 20 reset/load/lifecycle cycles on Android with graphics active; stale results never publish; explicit pending semantics and queue/staging/residency bounds. |
| D3.1 → D3.2 → D3.3 | Integrate representative detail/dynamic geometry into lighting; extend to moving lights, opened/closed enclosures, edits and reflectors; finish stable visual transitions and temporal correctness | Android captures demonstrate indirect response/color bleed, bounded leakage, reflection changes, no stale publications; declare response/latency and seam criteria. Evaluate reconstruction correctness if implemented; optional acceleration and cost ranking wait. |
| D4.2 → D4.3 | Complete only missing minimal animation/UI/assets and shared persistence/seeded behavior; document authoring and extension for both samples | Both samples run without engine forks; functional input, physical simultaneous touch, lock/unlock, audible output and persistence observed. |
| E8-T | Freeze integrated technical candidate and reproduce all affected capabilities | Clean checkout/APK identity, host/native checks, physical two-sample acceptance, resolved reviews, passing CI and merged PRs. |

The first slices alone do not close D1–D4. Each follow-on receives its own exact
base, nonoverlapping paths, observable DoD and Android handoff before dispatch.
No new renderer or service framework is justified merely by the plan.

## Dependency DAG and critical path (Phase A)

```
E0/E1 closed
   ├── D1.1 → D1.2 → D1.3 ──────────────┐
   ├── D2.1 → D2.2 → D2.3 ──────────────┤
   └── D4.1 → D4.2 → D4.3 ──────────────┤
D1/D2 minimal interface → D3.1 → D3.2 → D3.3
                                        ↓
                         integrated Android gates → E8-T
                                                      ↓
                     Phase B: E2 → E3-M + E5-M → E6/M5 → E8-F

```

Critical path is D1/D3/D4 → E8-T → Phase B → E8-F; E8-T precedes Phase B and E8-F
closes it. D2 feeds D3's publication semantics and D4's persistence evidence. Reassess
the path after each accepted slice; do not start Phase B work to fill idle time.

## PR slicing and integration

- One accepted vertical slice = one PR from `main` at the frozen base: D1–D4, then E8-T.
  Package PRs contain the code, tests, the worker log and necessary docs; the lead
  updates STATUS/ROADMAP/ADR/board in the integration PR or a separate lead PR.
- No mixed-package PRs, no drive-by refactors outside the brief, no generated binaries
  in Git. Large evidence lives in durable workflow/release artifacts with checksums;
  small manifests and summaries stay in Git.
- The lead reviews the diff and merges under existing delivery authorization after
  CI and required reviews pass. Behavioral changes receive the relevant combined
  Android acceptance; host-only groundwork carries explicit unrun device gates as
  described above. Preserve branch protections and history; no force-push.

## Independent review and correction gates

Gemini's six read-only scopes, executed in bounded waves alongside (not instead of)
required verification:

- G1 D1.1 probe determinism, observable criterion and the planned D1 slice sequence.
- G2 D2.1 discrimination between a stall and correct deferral; no stale-result merge.
- G3 D3.1 production geometry coverage and response criteria; fence-outlier deferral.
- G4 D4.1 audio/lifecycle seam and reuse/type-leak coverage.
- G5 E8-T evidence completeness, reproduce commands and unrun-gate transparency.
- G6 Phase B E2 qualification inputs before any cost ranking is trusted.

Findings are triaged by the lead; DeepSeek corrects in-scope items; only affected
scopes are re-reviewed. Reviewer agreement is not correctness. Opus escalation
requires a recorded blocker, prior attempts and why in-scope routes failed. At most
three workers (implementers + reviewers) run alongside the lead.

## Human, device and thermal gates

- **Phase A functional (immediate, not deferred):** keyguard unlock, both samples run
  on the OnePlus 13, input/controls functional check, Voxel Relay and wetland playback,
  and the M2–M4/M6 functional slices once integrated.
- **Human observations that counters cannot replace:** hearing confirmation for audio
  output (still open), physical simultaneous multi-finger touch (ADB gestures are
  sequential and are not this gate), and physical lock/unlock plus resume.
- **Phase A human/device gates are not deferred:** hearing confirmation, physical
  simultaneous multi-finger touch and physical lock/unlock are Phase A gates for M6.
- **Phase B only:** each matched pair requires fresh qualified thermal conditions
  (unplugged, cooled, idle-ready). No thermal or energy conclusion is drawn from Phase A.

## Measurement and decision rules (Phase B preserved)

Apply [BENCHMARKS](BENCHMARKS.md) and the [measurement qualification](performance/measurement-qualification.md)
contract. Freeze resolution, quality, scene, seed, replay, build and refresh policy.
Start with three alternating short A/B pairs; finalists require a documented warmup and
sustained runs per protocol, with repeats where variance matters. Existing working
targets p95 ≤16.67 ms and p99 ≤25 ms are targets, not achievements. Predeclare quality,
latency and memory thresholds without adding a fixed-resolution mandate or an invented
percentage ghosting acceptance; one causal change per comparison; apply the existing
improvement/no-regression rule; after two inconclusive iterations improve attribution
or change the measured cost. The original M2 equivalent-quality comparison, primary-path
selection and ADR-0005/0006/0007 resolution happen here with the Phase A provisional
path as a candidate, never as a pre-decided winner. Every run names an exact profile ID
and the default effort/quality setting; performance-only attribution of the lone
7.974 ms fence observation is deferred until it recurs or a cost decision needs it.
P02's thermal regression is retested
on the final equivalent profile; an unmet target stays open or receives an explicit
engineering revision preserving the original result. No commercial or UE5-parity claim
follows from any result.

## E8-T technical checkpoint, then E8-F final measured closure

E8-T (Phase A exit) requires: D1–D4 accepted; clean-checkout build and APK identity for
the exact candidate; Android functional acceptance of both samples including input,
lifecycle and persistence; Phase A proof is functional fixtures plus explicitly declared observable criteria, not fixed-resolution or invented-threshold conformance; frozen independent reviews with findings resolved; workspace
tests, fmt, strict Clippy, `python3 tools/check_docs.py` and CI green; all Phase A PRs
reviewed and merged; each gate records a verification result — PASS, PARTIAL with the
exact remaining gate, or NOT RUN — and no gate disappears inside a partial count;
STATUS/ROADMAP/ADR/board updated with exact commits, commands,
PASS/FAIL/NOT RUN and an explicit list of what Phase B still owns; large evidence in
durable artifacts with checksums. E8-T does **not** close the original measured
acceptance and must say so.

E8-F (Phase B exit) repeats E8-T on the measured candidate plus E2 qualification,
E3-M cost ranking and ADR resolution, the M5 sustained/thermal report and the P02
retest, and the final supported-limits table. Only E8-F can close the original M0–M6
acceptance. If phone, provider or review access is unavailable, retain the checkpoint,
record the exact blocker and continue independent work; ask the owner only for a
missing owner decision or inaccessible capability. Store publication, licensing and
purchases remain owner-controlled.

The final handoff must let a fresh session build the engine, run both samples,
reproduce Phase A acceptance, see exactly which Phase B gates remain and understand
every supported limit from this repository and its durable artifacts alone.

Worker routing records use `deepseek-flash-go` / `deepseek-v4.1-flash` / `max` and
`gemini-restricted` / `gemini-3.8-flash-high` / `high`, checked by Pi before inference.
Opus escalation uses `opus-medium`, or `opus-xhigh` for a consequential design
question, only with a recorded reason. Record worker acceptance separately through
`router.py verdict WORKER --status accepted|repaired|rejected|abandoned`; this does
not replace engine PASS/FAIL/NOT RUN gates. Unknown quota stays unknown; no automatic
model fallback or repeated full-context reviews.
