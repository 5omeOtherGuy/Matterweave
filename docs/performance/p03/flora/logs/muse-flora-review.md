# muse-flora-review — frozen candidate02

Read-only review; lead triage and repairs follow in execution_log.md.

Profile: {'provider': 'opencode', 'model': 'muse-spark-1.3-contributor-free', 'thinking': 'high'}; outcome completed; elapsed 94.65s.

Review of frozen P03 flora candidate (dense 16 m HOST fixture, seed 20260908). Tests NOT RUN.

**Issue 1 — load-bearing stems/culms outside checked footprint**
Path: `crates/matterweave-detail/src/flora.rs:628-644` (`support_height` filters `cell[1] != 0`), stems at `:208-211`, culms at `:391-404`.
Trigger: `clustered_mushroom` stipes at bases e.g. `[-5, -4]` (~0.28–0.40 m offset) and `reed_cluster` outer culms at `±2` cells (~0.31 m ≈ 1.25 tile cells) land 1–2 tile columns from the `y=0` root disc/pad, on creek-bank relief where neighbour columns differ by ≥1 tile cell (0.25 m).
Consequence: only the central disc/pad is verified flat moss; outer stipe/culm bases can float or bury while `gap_m=0` is reported. Collision then floats/sinks vs. walkable surface.
Evidence: contact set built from `y==0` cells only, by design comment `:607-614`; independent recheck in `crates/matterweave-detail/tests/flora.rs` (corridor/support test) uses the same `c[1]==0` filter, so it is blind to this.
Uncertainty: actual relief at chosen placements not executed; magnitude depends on neighbour height deltas in the canonical tile.

**Issue 2 — `instance_support` snaps fractional translations**
Path: `crates/matterweave-detail/src/flora.rs:662-678`, vs. foundation `crates/matterweave-detail/src/lib.rs` (`Transform` fractional translation explicitly supported).
Trigger: call `instance_support` with any translation not exactly on a tile-column centre (e.g. shifted 0.06–0.12 m by edit/tool). `cx/cz` are floored, then contacts are recomputed from `(cx+0.5)*tile_cell`, discarding the fraction.
Consequence: support columns, material, and `gap_m` refer to the snapped centre, not the actual instance — false rooted/unsupported and gap error up to half a tile cell.
Evidence: `wx = (cx+0.5)*tile_cell + rotated[0]` ignores `translation_m` fraction; `let _ = prototype_id` shows no per-prototype correction.
Uncertainty: `dense_tile` placements are centre-aligned, so the frozen manifest is unaffected; only general reuse (edits, native adapter) is wrong. Fix is doc-narrow ("column-centred placements only") or use true translation.

No third material issue found. Palette additivity, fungus-Collision vs. leaf-Decorative split incl. `FLORA_FUNGUS_LUMEN` vs `FLORA_LUMEN_DOT`, bracket-reserved error, corridor exclusion, bounded deterministic search with explicit per-species/global threshold failure all check out on read. Visual recognizability not judged — root examines renders.

Engineering log: Actions Taken — read `flora.rs`, `lib.rs` palette/Transform/Yaw, `tests/flora.rs`, `flora_gallery.rs`, `docs/performance/p03/{foundation,verification,gallery-acceptance}`; traced foot mapping, yaw formula, offsets, support API. Issues & Friction — none blocking; line refs via targeted grep, no broad search. Decisions & Rationale — scoped strictly to new flora; did not re-audit storage/mesh/query; did not judge art style; reported 2 concrete grounding/API issues, no filler third. Solutions Applied — none (read-only review). Insights — fixture's "fully supported" claim holds only for `y=0` centres; outer vertical structure and fractional-placement queries are the two seams to harden before native acceptance.
