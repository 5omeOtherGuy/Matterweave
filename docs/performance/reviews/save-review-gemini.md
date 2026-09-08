# Save restoration review and lead disposition

Source reviewed: `221f720`, read-only Pi Gemini 3.8 Flash high. The first120-second
attempt timed out after source reads; an exact-session, tool-free continuation
returned its findings in18.61s. Raw sessions remain in
`completion-02/save-review-gemini` and `save-review-gemini-finish`.

## Candidate findings

1. Restored sessions retained the raw generator entrance in `Runtime.spawn`;
   only fresh sessions applied its bounded collision-clearance lift. Lead
   confirmed this matters now: both `Action::Home` and the below-world respawn
   path use `r.spawn`. The review incorrectly suggested no current frame-loop
   consumer. Fixed by carrying a validated spawn with prepared session state.
   If source edits block every bounded entrance pose, use the already validated
   saved viewpoint as the respawn point. Preserve the actual saved camera eye.
2. Entrance fallback preserves saved yaw/pitch. Lead retained this intentionally:
   journal orientation remains authoritative, consistent with ordinary Home
   behavior. The correction is logged and normal look controls remain available.
   No requirement calls for resetting orientation; this is not treated as a bug.

The reviewer reported no rejected-source leakage, body contamination, save-byte
corruption or repeated preparation issue. These are source-review observations,
not execution or final approval. Lead inspected the actual branches/contracts.

## Verification and engineering log

- Actions: inspect source isolation, real restoration, recovery precedence and
  prepared ownership transfer; verify each candidate against actual consumers.
- Issues: initial review timebox expired; completed using only already-read code.
  The respawn regression (`48ac799`) first failed compilation because prepared
  state did not carry a validated spawn. No fictional historical test result.
- Decisions: retain bounded collision clearance and saved orientation; use saved
  viewpoint as respawn only when the edited entrance has no clear bounded pose.
- Solutions: regression covers lifted entrance and completely blocked entrance,
  asserting saved eye preservation and actual teleport acceptance. All64 app
  tests, including the normally ignored full-map Runtime test, passed in17.33s.
- Insights: validating the current eye alone is insufficient when later actions
  also depend on a stored respawn pose. Source review can find consumers missed
  by the reviewer itself; lead verification remains necessary.

Native Vulkan replay/capture at the preceding source passed25 frames/20 valid
rows/six bodies/no validation errors. That is not phone route or final-build
acceptance. Strict lint and final Android delivery remain lead-owned.
