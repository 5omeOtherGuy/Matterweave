# App interaction/capture audit

Frozen source 9b7faa9; Pi Astra medium, read-only.

### Actions
Read the three scoped files; cross-read detail edit/collision transactions, physics restore, and GPU diagnostics/completion handling. No modifications.

### Issues
1. **P2 — Placement can select a diagonal cell.**  
   `apps/explorer/src/wetland_state.rs:259–262` records the previous cell, then advances **every** tied crossing axis. `wetland.rs:210–212` uses that previous cell for placement without checking face adjacency.  
   **Trigger/proof:** Within prototype bounds, a ray crossing an XY corner from empty `[0, 0, 0]` into solid `[1, 1, 0]` returns `[0, 0, 0]` as `previous`; Manhattan distance is two.  
   **Consequence:** Place adds an edge-adjacent voxel rather than one on the aimed solid’s face.

2. **P2 — Paused main menu still accepts world-edit keyboard shortcuts.**  
   `apps/explorer/src/wetland.rs:949–951` dispatches actions without checking `menu`; `action()` at `:478` checks only runtime existence.  
   **Trigger:** Aim at reachable terrain, return to the main menu, press **R**.  
   **Consequence/proof:** The retained runtime executes Remove behind the opaque menu; normal exit saving (`:1010`) persists the hidden edit.

### Decisions
Excluded both acknowledged save-validation/pose gaps. No new GPU-join defect identified: recorded-submission filtering and epoch-qualified identities appear consistent with the one-in-flight contract.

### Solutions
- Enforce deterministic face adjacency at internal DDA ties; add a corner-crossing regression.
- Gate gameplay shortcuts while the main menu is active; test that paused shortcuts leave the edit journal unchanged.

### Insights
Collision replacement prepares before committing; rollback restores cell content. Findings above are source-derived, not reproduced.

**Checks not run:** Builds, tests, shell/Git revision verification, device interaction, and capture replay. Prior evidence was supplied, not independently rerun. Lead owns acceptance.

Lead: both findings reproduced under tests at cecd895. Correction gates pass;
no new GPU capture defect was identified.
