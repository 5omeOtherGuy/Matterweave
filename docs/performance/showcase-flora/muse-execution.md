# Muse worker execution log — wetland support flora

Task: 3 new non-fungal support-flora prototypes (twisted woody shrub, tall
horsetail spire, broad water lily) bringing existing 3 support flora to 6.
Bracket fungus RESERVED (Hy4/lead) — not implemented. No new external
dependencies; existing flora and palette untouched.

## Actions Taken (all executed, chronologically)

1. Read `docs/SHOWCASE.md`, `crates/matterweave-detail/src/{lib,flora}.rs`,
   `tests/flora.rs`, `examples/flora_gallery.rs`, `src/serial.rs`.
2. Created `crates/matterweave-detail/src/wetland_flora.rs`:
   `twisted_shrub`, `horsetail`, `marsh_lily`, `wetland_prototype`,
   `WETLAND_FLORA_SPECIES`.
3. Minimal `lib.rs` edit: `mod wetland_flora` + re-exports only. No palette
   or existing-flora changes.
4. Created `tests/wetland_flora.rs` (8 tests) and
   `examples/wetland_flora_gallery.rs` (manifest + LE f32/u32 bins for
   source/half/quarter + snapshots + host-only timing, mirroring
   `flora_gallery.rs` layout).
5. Ran `cargo test -p matterweave-detail` (private target
   `/mnt/bench/matterweave-dev/performance/completion-01/flora-target`,
   `CARGO_BUILD_JOBS=2`): 58 passed, 5 suites.
6. Ran gallery export to
   `/mnt/bench/matterweave-dev/performance/completion-01/flora-gallery-wetland`.

## Real measured counts (from gallery manifest, not test claims)

| prototype | cell m | occupied | policy split | source verts/tris (LOD1/2/4) |
|---|---|---|---|---|
| twisted_shrub | 0.125 | 497 | 141 collision / 356 decorative | 2828/1414, 936/468, 288/144 |
| horsetail | 0.0625 | 720 | 720 decorative | 5184/2592, 1860/930, 328/164 |
| marsh_lily | 0.125 | 219 | 219 decorative | 1508/754, 576/288, 192/96 |

All < 33,288-cell cap; unique stored cells total 1430; 9 derived mesh builds.
Bounds (local m): shrub x[-1.125,1.5] y[0,2.5] z[-1.0,1.25];
horsetail y[0,2.125] (~1.9 m spire + cone); lily x[-1.375,1.375] y[0,0.75].

## Decisions & Rationale

- Wood reuses `FLORA_FUNNEL_STIPE` (ID 30, Collision, warm tan): existing
  collidable wood-tone solid, so trunks stay gameplay walls and read as wood.
  `BANK_STONE` rejected (gray terrain stone, would misread as rock). Documented
  in module docs and gallery manifest. No palette IDs added.
- Lily fully decorative, floating pads at local y = 2; dry-moss
  `instance_support` explicitly does NOT apply — water placement recorded in
  manifest (`water_placement` field).
- Lily pads alternate sides along an x-runner (bounded integer layout, radii
  3–4, notch slit dx>0/z==0); minor 1-cell graze between two pads accepted as
  a natural colony touch, centers stay distinct.

## Issues & Friction

- First run: lily 213→(threshold self-check) under my own 200-cell
  placeholder bar only after layout change; fixed by widening runner to
  -7..7 and radii pattern [4,3,3,3,4].
- Cone-tip counting via "plume cell with air above" gave 33, not 3 (counts
  every cone surface cell). Fixed by counting plume-only 6-connected masses.
- Added second lily bud with petals but no center cell → 5 components, caught
  by `debug_assert` + connectivity test. Fixed by adding the y=4 center petal.

## Solutions Applied

- See above; all fixes verified by re-running the full suite (58 passed) and
  re-exporting the gallery.

## Insights

- Plume/cone surface cells make bad tip counters; connected-mass counting over
  one material is the robust shape assertion.
- `debug_assert_eq!(components, 1)` inside builders catches detached anatomy
  at construction time in test builds without affecting release.

## Remaining / NOTRUN

- Anatomy/readability lead acceptance (in-engine front/side/underside views):
  NOTRUN — lead owns host views from the exported LE bins + manifest.
- No full-map scatter or physics/app changes (per scope).

## Lead review corrections
- **Actions Taken:** Independent Muse and Gemini reviews completed before lead code inspection. Fixed branch side taper to be perpendicular on both axes; corrected lily notch test to check the actual slit and flanking walls; labeled the mixed-policy shrub flora-woody and clarified source-only snapshots.
- **Issues & Friction:** Original notch test passed on incidental inter-pad gaps. Existing flora_class default mislabeled collidable shrub wood.
- **Decisions & Rationale:** Debug-only connectivity assertions are not a defect for these fixed deterministic builders with release geometry independently checked; no runtime scan added just to repeat tests.
- **Solutions Applied:** Narrow geometry/test/export fixes; regenerate gallery and verify before native integration.
- **Insights:** Tests of appearance need authoritative feature locations, not any matching voxel somewhere in the bounding box.
