# map-muse-finish

Frozen 952d630; independent read-only candidate findings.

Review — `completion-map` frozen `952d630` (unchanged, no edits/shell/tests).

Scope only: `crates/matterweave-detail/src/showcase.rs`, `crates/matterweave-detail/tests/showcase.rs`, `crates/matterweave-detail/src/lib.rs` exports.

Up to 3 correctness candidates (not verdicts):

1. `showcase.rs:114 source_radius_m` vs `showcase.rs:route_clearance` / `tests/showcase.rs:plant_source_geometry_clears_both_routes…`
   - Trigger: archetype with extent in both x and z places origin outside `ROUTE_PLAYER_CLEARANCE_M + radius_m` but diagonal corner still intrudes.
   - Consequence: cap/branch in walked corridor; route-clearance test failure.
   - Evidence: radius = `fold(max)` over axes `0, 2` (Chebyshev); clearance/test use Euclidean `dist2/sqrt`; both routes are combined into `corridor`, so scope is correct, metric is the candidate.
   - Uncertainty: prototype bounds in `flora.rs`/`wetland_flora.rs`/`bracket.rs` out of scope; shortfall `0` to `~0.41x` unquantified; no seed measurement.

2. `showcase.rs:1128 HABITAT_*` + `showcase.rs:place_flora` + `showcase.rs:build_showcase` species check vs `tests/showcase.rs:canonical_counts_meet_showcase_targets`
   - Trigger: habitat-gated random pick + water/slope/overhung/material/route/footprint rejections starve a habitat-narrow archetype (e.g. waterside-only, grove-only).
   - Consequence: build `BudgetExceeded(an archetype was not placed)` or test `>=100 per species` / `len==10` / woody/fungus exact-sum failure while aggregate density passes.
   - Evidence: union covers 9 + separate lily sweep; per-instance choice is `palette[hash%len]`; build only checks `species.len()==10`; test requires `100+` each and class exact equality.
   - Uncertainty: actual `species_instances` on frozen seed not observed (read-only, no run); cannot confirm starvation.

3. `showcase.rs:420,461 Terrain::generate overhung` + `showcase.rs:build_terrain_voxels` roof count vs `tests/showcase.rs:surface_query_reports_real_solid…`
   - Trigger: collidable flora cell overlaps `[x,cy,z]` segment `surface.top+1..=phantom_top` (lowered mouth) or void `(0..top)` (roofed column).
   - Consequence: `is_collidable_world_metres` true above surface / no air below roof, even if terrain voxels agree with query.
   - Evidence: that test uses global `is_collidable` without `i_shell_/base_` filter (unlike `surface_query_agrees…`); `place_flora` only refuses `overhung` footing, does not reserve neighbouring airspace; `lib.rs:material_policy` makes fungal/woody parts `Collision`.
   - Uncertainty: overlap on frozen map unobserved; prototype extents/placements out of scope. Do NOT infer this caused the reported `surface_query_reports_real_solid…` failure/worker timeout; full `512x512` scan cost also unevaluated.

Engineering log:
- Scope: showcase.rs + tests/showcase.rs + lib.rs exports only.
- Evidence: code paths above read in-session; no runs/edits.
- Analysis: 3 candidates with metric/coverage/interference risks.
- Verdict: no substantiated failure cause; candidates only.
- Follow-up: needs owner-run counts + targeted terrain-only vs global query comparison.

Coverage gaps: flora/wetland/bracket prototype geometry; `DetailScene::{counts,sample_*,is_collidable}` semantics; `952d630` vs `5fe275f` diff; measured manifest/counts/hashes; performance of full-column cavity test.


## Lead verification

Diagonal source radius and between-vertex route intrusion reproduced by actual-scene regressions; corrected without reducing density gates. Cavity overlap reproduced at [97.375, 16.125, 58.125] as parasol cap material 21, not terrain: corrected terrain-only assertion passes 610 lowered columns and 2856 overhang columns. No product requirement forbids fungi on cavity floors; retaining them is intentional. Claims of measured counts in a read-only reviewer summary are not execution evidence.
