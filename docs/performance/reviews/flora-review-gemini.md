# flora-review-gemini

Frozen source reviewed independently; candidate findings, not acceptance.

### Engineering Handoff Log

- **Accomplishments**: Completed independent read-only review of `matterweave-detail` wetland flora additions (`wetland_flora.rs`, `tests/wetland_flora.rs`, `examples/wetland_flora_gallery.rs`, and minimal lib exports) across frozen revision `a896860` vs `c6e2317`.
- **Commands / Verification**: Code inspection only via file reads. Tests NOT RUN.
- **Key Findings**: All three prototypes (`twisted_shrub`, `horsetail`, `marsh_lily`) meet cell budgets (≤33,288), explicit scale constraints (6.25 cm / 12.5 cm), palette preservation, 6-connectivity, and wood collision vs. leaf decorative policy. Three concrete gaps identified in test geometry assertion, branch lateral taper logic, and manifest classification export.
- **Risks**: Visual aesthetics and terrain integration cannot be inferred from tests; full-map scatter and phone frame-rate/memory stability remain unmeasured pending lead native integration.
- **Next Actions**: Reorient notch assertion axis, make shrub branch taper orthogonal to branch direction vector, and correct manifest class export before device profiling.

---

### Candidates

1. `crates/matterweave-detail/tests/wetland_flora.rs:240`
- **trigger**: `test_lily_has_flat_notched_pads_and_a_flower` checks for an air cell flanked along X (`[x-1, 2, z]` and `[x+1, 2, z]` non-air) to assert `notch`.
- **consequence**: In `marsh_lily`, the radial notch slit is open to the outer rim along +X (`x > 0 && z == 0`), meaning pad walls flank Z (`z ± 1`), not X. The test evaluates the wrong axis and passes vacuously on an incidental 1-voxel air gap between separate pad discs (Pad 1 and Pad 3 at `x=2, z=-4`). The test provides false coverage and still passes if the notch cut is removed.
- **evidence**: `wetland_flora.rs:401` cuts slit via `if x > 0 && z == 0 { continue; }`. In `tests/wetland_flora.rs:237-245`, Pad 1 (max x=1 at z=-4) and Pad 3 (min x=3 at z=-4) create an air cell at `[2, 2, -4]` between non-air neighbors.
- **uncertainty**: None.

2. `crates/matterweave-detail/src/wetland_flora.rs:188`
- **trigger**: Shrub branch taper voxel is hardcoded along +Z (`side = [cell[0], cell[1], cell[2] + 1]`) regardless of branch takeoff direction `dir`.
- **consequence**: For Z-oriented branches (`dir = [0, 1]` and `[0, -1]`), the side cell advances longitudinally along +Z or back into the trunk at `z=0` instead of expanding lateral width along X. Z-branches do not achieve the intended 2-cell takeoff taper.
- **evidence**: `SHRUB_BRANCHES` defines `(9, [0, 1], 6)` and `(11, [0, -1], 5)` in `wetland_flora.rs:139-140`. Lateral expansion requires `[-dir[1], 0, dir[0]]`.
- **uncertainty**: None.

3. `crates/matterweave-detail/examples/wetland_flora_gallery.rs:74,111`
- **trigger**: Gallery export calls `flora_class(&id)` for wetland prototypes.
- **consequence**: `flora_class` (in `flora.rs:71-77`) only recognizes fungi and terrain, defaulting all others to `"flora-decorative"`. `twisted_shrub` (containing >100 collidable wood voxels) is exported as `"flora-decorative"` in `manifest.json`, misclassifying collidable obstacles for downstream consumers.
- **evidence**: `flora.rs:76` has `_ => "flora-decorative"`.
- **uncertainty**: Low; `cells_by_policy` preserves raw counts, but top-level `class` string is inaccurate.
