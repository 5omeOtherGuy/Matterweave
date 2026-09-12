# completion-plan-v2-deepseek — owner-reprioritized completion plan (planning log)

Scope: rewrite `docs/ENGINE_COMPLETION_PLAN.md` into a Phase A technical-completion /
Phase B measured-closure plan after the owner priority change, and record the routing
revision. Documentation only: no engine build, APK, phone access, worker dispatch or
code change was performed.

Workspace: `/mnt/bench/matterweave-dev/worktrees/completion-plan-v2`
Base: `01ab450a80a2aa5069321cdec7328e628faeae8b` (PR #21 merge, `main` = `origin/main`).

## Actions Taken

1. Read AGENTS, HANDOFF, STATUS (reconciliation wave, E1 audio/cadence, reflections,
   P02, open-gate sections), REQUIREMENTS, ROADMAP, ADR index, the gate ledger
   `docs/performance/completion-gates-20260912.md`, `measurement-qualification.md`,
   `reflections-engine.md`, and the old plan; verified the frozen base with
   `git rev-parse` and an empty `git status --porcelain`.
2. Confirmed the review corrections against source rather than summary:
   `crates/matterweave-render/src/indirect.rs` documents "Unit voxel geometry only:
   detail meshes and dynamic mesh-only objects ... are not supported";
   `crates/matterweave-render/src/reflection.rs` states dynamic meshes, static
   instances and detail geometry are nonreflective. `apps/explorer/Cargo.toml` has no
   `matterweave-audio` dependency.
3. Rewrote the plan (250→~280 lines) with Phase A (M2/M3/M4/M6 technical completion,
   E8-T checkpoint, Android functional tests as slices land) and Phase B (E2
   qualification, equal-quality cost ranking and primary-path selection, M5
   sustained/thermal, E8-F) so technical completion is visibly distinct from the
   original measured acceptance. M5 is marked deferred by the owner, not passed.
4. Made D3 concrete: production detail/moving-object lighting integration, citing the
   `indirect.rs`/`reflection.rs` limits above. Made D4 concrete: real audio/lifecycle
   integration into `apps/explorer`. Added D2 exclusions: the body-overlap starvation
   candidate is unverified and must be reproduced or dismissed first; no stale-result
   merging; no invented mandatory `EngineContext`.
5. Removed E2 from every Phase A prerequisite and dispatch/acceptance path; deferred
   thermal to Phase B with fresh qualified conditions per matched pair, and removed
   battery/charge blocking from Phase A per the owner.
6. Added the Phase A quality bar: functional fixtures plus explicit observable declared
   criteria; no fixed-resolution (1080p) mandate and no invented 1% ghosting threshold.

## Issues & Friction

- The owner changed priorities mid-task: the first draft ordered E2 before
  implementation. The revision removes that dependency entirely rather than listing it
  as a soft gate, because a "soft" prerequisite would still block dispatch.
- The six-family campaign mandate in HANDOFF/PERFORMANCE_PLAN conflicts with the
  current routing. The plan records the supersession in its own routing section; those
  source documents were not edited (out of the owned-file scope) and still need the
  lead's update.
- `docs/performance/board.json` and many `/mnt/bench` worktrees still name historical
  owners. The plan keeps the board as the sole live assignment authority (plan tables
  are planned only) and forbids resurrecting stale work.
- The old plan fitted 177 lines; the reprioritized version is longer while staying
  under the 350-line limit, so some v1 detail (wave narrative) was folded into tables.

## Decisions & Rationale

- Phase split is the owner's: capability completion now, measurement later. M5 is not
  silently dropped; it is deferred and stays open in the map and E8-F.
- Phase A packages D1–D4 start from frozen `01ab450`; E2 work cannot precede them.
- D1/D2/D3 remain host-first because workers never get device access; the lead runs
  Android functional slices immediately after integration, which is what "functional
  tests now" can mean without violating the ownership split.
- D3's scope is set by source evidence, not by the previous "functional response"
  wording alone: unit-voxel-only indirection and non-reflective detail/mesh surfaces
  are the actual missing capabilities.
- D4 is an integration task, not an audit: the explorer has no audio dependency at all.
- Attempt budgets are 900 s implementation and 240 s read-only review per 1-PR slice,
  with at most one same-scope correction before resplit or an evidenced Opus escalation;
  they are dispatch budgets, not wall-clock completion promises.
- Model routing: DeepSeek V4.1 Flash implements; Gemini reviews read-only in six
  bounded scopes; Opus only with a recorded blocker; no Astra in any role; model
  agreement is never a correctness or score claim.

## Solutions Applied

- Introduced explicit Phase A / Phase B gate columns in the milestone map and a
  separate Phase B package list (E2, E3-M, E6/M5, E8-F) so no reader can mistake
  E8-T for the original M0–M6 closure.
- Wrote Phase A exclusions into the briefs themselves so an implementer cannot
  "fix" the unverified starvation candidate by merging stale results or by
  introducing a speculative `EngineContext`.
- Kept the human gates (hearing, simultaneous touch, lock/unlock) and the fence-outlier
  hypothesis (7.974 ms single observation) explicit in both plan and briefs.

## Insights

- The earlier plan's E2-before-E3 ordering made a device measurement gate the
  critical path for renderer work; the reprioritization shows most correctness and
  representation work never needed it. Device-dependent work should name its device
  dependency at the task level, not gate the whole milestone.
- Lighting "done" at unit voxels is not production lighting; source-code capability
  statements (comments plus bindings) were a faster and more reliable gap inventory
  than milestone prose.

## Verification

- `wc -l docs/ENGINE_COMPLETION_PLAN.md` → under the 350-line limit.
- `python3 tools/check_docs.py` → PASS (recorded in the handoff).
- `git status --porcelain` → only the two owned files changed; no commit or push.
- Named commands/facts were re-checked against source; no device, build, thermal or
  timing result was executed or claimed for this plan.

## Definition of done (worker portion)

| Criterion | Verification method | Owner | Result |
| --- | --- | --- | --- |
| Complete milestone/gate/requirement mapping with real dependencies | Cross-check ROADMAP/REQUIREMENTS/ADRs; Phase A/B columns | worker | done |
| Current routing/state and 3 dispatchable tasks | Read STATUS/source; D1.1–D4.1 first slices with paths, interfaces, DoD, stop rules and Pi budgets | worker | done (D1.1, D2.1, D4.1 wave 1a; D3.1 next slot) |
| Documentation valid | `python3 tools/check_docs.py` | worker | done (PASS) |
| Independent reviews and integrated final plan | Gemini review wave and lead integration | lead | NOT RUN |
