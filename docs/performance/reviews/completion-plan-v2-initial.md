# Initial technical-completion review triage — 2026-09-12

Frozen code: `01ab450`. Six independent Gemini/Pi scopes inspect candidate gaps;
DeepSeek/Pi authors the plan. No engine code or device verification is performed
in this planning change. Raw sessions: `/mnt/bench/matterweave-dev/completion-plan-v2`.
This compact record is durable; raw model assertions do not establish defects.

## Lead-verified disposition

| Scope | Candidate | Disposition and next action |
| --- | --- | --- |
| Renderer/lighting | Comparison is a small unlit reference, not a production-quality winner | Confirmed in `renderer_comparison.rs` and existing comparison report. Preserve reference; integrate functional production capabilities now, measured selection later. Reject invented 1080p and 1% ghosting requirements. |
| Renderer/lighting | Unit-grid GI/reflection excludes fine/detail/dynamic source geometry | Confirmed explicit limits in `indirect.rs:67` and `reflection.rs:53`. Production lighting coverage is a development task. Reviewer claim that all detail cannot receive any indirect light is broader than established: `world.wgsl:246` samples indirect shading. Separate receiving shading from participating in traced geometry. |
| Core/services | Audio absent from application | Confirmed `apps/explorer/Cargo.toml` has no audio dependency. Implement shared lifecycle integration and bounded sound events for both samples. Diagnostic counters are not audibility. |
| Core/services | Streaming stale-result rejection starves under continuous edits | Rejection confirmed at `async_world.rs:245`; starvation not reproduced. Add adversarial progress test before changing policy. Never merge stale snapshots merely to make progress. |
| Core/services | Streamed bodies fall through evicted terrain | Not established: `Physics::step` disables unsupported bodies using resident columns (`lib.rs:325–350`); app calls `sync_world` after `poll_stream` (`apps/explorer/src/lib.rs:1024`). Preserve safeguards and test integrated transitions. No mandatory new EngineContext abstraction. |
| Android functional gates | Thermal collector blocks functional progress | Wrong tool for this purpose, not a collector defect. Existing `engine-check.txt` detail/indirect/reflection and standalone audio gates need no thermal campaign; use DEVELOPMENT commands. A short runner is optional convenience, not an engine milestone. |
| Android functional gates | Route script requires unplugged device | Confirmed `check_phone_route.py:125`; scope any relaxation to explicit functional mode, preserving strict performance collector contracts. Not required to start other device gates. |
| Detail | Automatic LOD absent everywhere | Overbroad: `detail_check.rs` has the runtime adapter and device gates. Production `wetland::graphics` currently builds Source meshes; extend/reuse the proven adapter for production instead of rebuilding automatic selection. |
| Detail | Edited prototype retains selection hysteresis | Candidate only. `choose_lod` rechecks updated dilation/local-loss before hysteresis, so no proven thin-feature bypass. Reproduce a violation of declared transition criteria before resetting state. |
| Detail | Coarsening drops minority materials | Expected possible loss in derived geometry, not automatically a world-authority defect. Validate visual criteria and existing conservative selection guards; preserve source truth. |

## Supervision

Initial reviews are candidate generation, not final acceptance. Several reviewers
exceeded the requested read/output bounds; narrow subsequent scopes and stop once
useful evidence is collected. Do not count tool calls as distinct file reads.
Worker identities and final acceptance dispositions are recorded on the board.
Further implementation reviews freeze each PR's source and examine its actual diff;
this initial review does not approve future code.

## Final two initial scopes

- **Services:** confirmed `Experience` is the shared lifecycle seam and audio is
  absent from app dependencies. Use existing service API and explicit real Android
  backend selection. Reject the review's categorical claim that Cargo cannot use
  target-specific dependency features; select and verify configuration in the actual
  app build. Do not copy suggested feature syntax without checking it.
- **Physics:** 64-body cap is already checked before fracture mutation. Review's
  claim that equal child angular velocities intrinsically inflate energy is wrong:
  rigid-body decomposition preserves angular velocity with orbital velocity; radial
  separation impulse is the intentional extra energy to document. Restore revision
  advancement is already invalidation, not proof a new epoch API is needed. Flight
  grab behavior remains an unverified candidate, requiring caller/behavior checks.

No speculative physics change is authorized by reviewer agreement. D2 closes
functional stress and concrete reproduced failures, preserving existing protections.
