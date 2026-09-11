# Engine-03 detail log: local interior-loss guard

Leaf worker scope: `matterweave-detail` code/tests only, plus this log and
[detail-local-loss-guard.md](../detail-local-loss-guard.md). No
renderer/app/physics edits. Renderer adapter and Android captures are lead-owned.

## Actions

- Added failing fixtures first (`crates/matterweave-detail/tests/local_loss.rs`):
  through-tunnel in a 16-cell solid, thin stem/sheet, adjacent instances/seam,
  dense safe block, perspective + orthographic zoom, source-invariance,
  config validation. RED: did not compile (`LodConfig` had no
  `max_local_loss_fraction`); tunnel-hold assertions then failed on old code path.
- Measured the existing coarsening before choosing the algorithm (throwaway
  probe over public `iter_cells`, deleted after; values in the guard doc):
  tunnel/pinhole global dilation 0.0029–0.0039 vs default bias 0.45 (passes —
  the admitted hole), worst interior-cell loss 0.25 at factor 2; dense solids
  read exactly 0 on both; stem/sheet read 0 locally but 0.50–0.94 globally.
- Implemented the minimal complement in `src/select.rs` / `src/scene.rs`:
  `ErrorMetrics::local_loss_fraction` (worst `1 - count/factor^3` over occupied
  coarse cells whose footprint lies fully inside the occupied cell bounds),
  `LodConfig::max_local_loss_fraction` (default `0.0`, validated `0.0..=1.0`),
  same break-out-of-coarsening-walk integration as the dilation bias, computed
  once per source revision in the digest cache.
- Updated `tests/lod_correctness.rs`: the pinhole test that pinned
  coarsening-away as documented behavior now pins holding at `Source`, with a
  relaxed-guard control proving which guard held it.
- Verification: `cargo test -p matterweave-detail` → 111 passed (14 suites);
  `cargo clippy -p matterweave-detail --all-targets -- -D warnings` → clean.
  All builds with `CARGO_BUILD_JOBS=1` and
  `CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/engine-03/detail-target`.
- `python3 tools/check_docs.py` → pass (see Handoff).

## Issues

- One new-test assertion initially expected a tunnel void to sample as
  `Some(AIR)`; `sample_world_metres` returns `None` on a miss. Fixed the test,
  not the source.
- Strict Clippy flagged `let mut dense = ||` (`unused_mut`) in the new tests.
  Fixed.

## Decisions

- Worst-cell (max) loss, not averaged loss: a 12-cell pinhole in 4084 cells is
  invisible to any average but reads 0.25 in the worst interior cell.
- Default `0.0` (any interior heterogeneity holds): conservative by design;
  enclosed voids trip it exactly like through-tunnels. Relaxation is per-caller
  via `max_local_loss_fraction`, not a global force-Source.
- Boundary coarse cells excluded via footprint-in-bounds check, so ordinary
  surface steps on dense geometry do not trip the guard (solids still coarsen).
- New `LodConfig`/`ErrorMetrics` fields ride the existing `..Default` /
  constructor sites only; every in-repo struct literal already used functional
  update, so no app/renderer edit was needed.

## Solutions

- Tunnel/pinhole held at `Source` far away (perspective and orthographic);
  relaxed guard (`1.0`) recovers coarsening as control.
- Dense 8-cell block still selects `Quarter` far / `Source` near, perspective
  and orthographic; adjacent touching instances both coarsen, seam stays
  collidable.
- Selection + preparation leave revision, snapshot runs and occupied counts
  unchanged; collision still reads the authoritative source.

## Insights

- The two guards are complements, not substitutes: interior-loss sees tunnels
  in bulk (dilation ~0.004, silent) while dilation sees isolated thin
  stock (local loss exactly 0, fully occupied clipped footprint). Either guard
  alone leaves a blind spot; the fixtures tested here resolve correctly with
  both. Anything untested is unclaimed — round 2 below is what happens when
  that scoping is taken seriously.
- Cost is one 4-byte count payload per coarse entry in the per-revision
  digest (entries bounded by the occupied-cell count; node overhead and
  padding are additional allocation under the same entry-count bound, not
  byte-exact) plus one footprint check per occupied coarse cell per factor;
  per-frame selection stays a table lookup.

## Round 2: boundary-notch blind spot (lead review)

- RED: added `boundary_face_pit_is_held_at_source_far_away` (2-cell pit in a
  13-cell solid face; old code selects unsafe `Half`, dilation ~0.20) plus
  `unaligned_dense_cuboids_still_coarsen_including_negatives` (odd-edge
  positive/negative dense controls reach `Quarter`, thin sheet stays held).
  Verified RED (`left: Half, right: Source`), committed as `7a298a6` before
  any source change.
- Fix: `worst_interior_loss` now evaluates `(expected - count) / expected`
  against each footprint *clipped to the occupied integer bounds* instead of
  skipping non-fully-inside footprints. Unaligned dense cuboids still read
  exactly `0`; the face pit reads `0.5` at factor 2 (`0.125` at factor 4).
- GREEN: `cargo test -p matterweave-detail` → 113 passed (14 suites);
  strict Clippy clean. Committed as checkpoint (see handoff).
- Doc corrections in the same pass: payload-vs-allocation wording (4-byte
  count payload per entry; node overhead/padding additional under the same
  entry-count bound), single-cell void fraction `1/f^3` (was misstated as
  `1 - 1/f^3` in `select.rs` docs), and `together` claims rescoped to tested
  fixtures only.

## Round 4 — local topology gate for the default guard (worker, from 1a42f59)

Task: fix the default guard's blanket rejection of organic/stepped geometry
while preserving local passages. Source/tests/docs in `matterweave-detail`
only; no app/renderer/physics changes.

### RED (commit `ec11bbe`)

`cargo test -p matterweave-detail --test local_loss` → 10 passed, 3 failed:

- `stepped_wedge_surface_coarsens_under_the_default_guard` — left `Source`,
  right `Quarter` (F1: a safe 45° stepped wedge, `z <= x`, pinned at Source).
- `prototype_edit_carving_a_channel_reverts_the_wedge_to_source`
- `instance_edit_holds_only_the_edited_instance_at_source`

Also added, passing at RED time (characterization, not a defect claim):
`material_filled_channel_is_outside_the_occupancy_guard` — a `WATER`-filled
channel coarsens while the same geometry in `AIR` holds `Source`. This scopes
the guard to occupancy explicitly rather than silently.
`unaligned_dense_cuboids_still_coarsen_including_negatives` now builds a fresh
scene per projection so hysteresis cannot carry the perspective selection into
the orthographic assertion.

### GREEN

`worst_interior_loss` now gates each partially filled coarse cell through
`local_fill_destroys_feature`: a `(factor + 2)^3` window (footprint + one-cell
halo), fixed stack scratch, no heap, no flood outside the window. A cell
contributes loss only if the fill closes a passage between halo air sites, or
buries an enclosed cavity or a pit (air site with ≥4 solid face neighbours).
Reuse checked first: the criterion is the block generalization of the
simple-point test from 3D thinning (Bertrand/Malandain); no Rust crate exposes
it outside a whole meshing/skeletonization engine, so ~60 lines here beat a
dependency. Budget `LOCAL_TOPOLOGY_CELL_BUDGET = 8192` analyzed cells per
(revision, factor); past it a cell keeps the old conservative verdict, which
can only hold a finer level. Cached exactly as before, by source revision.

Verification (`CARGO_BUILD_JOBS=1`,
`CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/engine-03/detail-target`):

- `cargo test -p matterweave-detail` → 118 passed, 0 failed across 15 targets
  (local_loss 13, detail 34, showcase 20, flora 15, lod_selection 15, …).
- `cargo clippy -p matterweave-detail --all-targets -- -D warnings` → clean.
- Measured cost, `scene::local_topology_cost` unit test (`--nocapture`), real
  prototypes: terrain_detail_tile 1488/535 analyzed cells and 95232/115560
  site visits at factor 2/4; parasol_mushroom 152/44 (9728/9504);
  funnel_mushroom 192/59 (12288/12744); fan_frond 136/49 (8704/10584);
  reed_cluster 108/29 (6912/6264). `budget_exhausted_cells == 0` everywhere —
  no real fixture falls back.

### Honest limits

- Occupancy only: material-heterogeneous channels remain unprotected
  (documented + pinned by test, not fixed).
- Rough organic surfaces (terrain tile, flora) still read high loss and stay
  at `Source`: their 1-cell crevices/pits are genuine features under the
  ≥4-solid-neighbour rule. Smooth stepped/sloped/wedge surfaces are the class
  unblocked this round.
- No device, Android or on-screen temporal-quality evidence: NOT RUN. All
  figures above are host `cargo` results on this worktree.

## Round 5 — joint closure of straddling channels (worker, from 11f70da)

Lead's counterexample (`detail-joint-red.log`): a 2x2 through-channel at
`y, z ∈ {7, 8}` straddles the factor-2 coarse boundary, so each single coarse
fill leaves a bypass through a neighbour and the round-4 per-cell test reads
`0.0`; the fills together erase the passage. RED reproduced at 11f70da
(`Quarter` vs `Source`).

### Repair

`local_fill_destroys_feature` now evaluates cells as a *sequence* in the
deterministic lexicographic `BTreeMap` key order: a window site is virtually
solid if the source is solid **or** its coarse key is an occupied coarse cell
ordered before the current one. "After" adds the current footprint, as before.
The accumulated end state is the full coarse fill, so the per-step checks
decompose the whole transformation instead of comparing each cell to the
untouched source. No extra state (membership in the existing `counts` map),
no second pass, no source mutation, same fixed stack scratch, same
`LOCAL_TOPOLOGY_CELL_BUDGET = 8192` budget and same revision-keyed cache.
`div_euclid` bucketing keeps the ordering correct on negative coordinates.

### Verification (`CARGO_BUILD_JOBS=1`, target `engine-03/detail-target`)

- `--test local_loss` → 17 passed, 0 failed, including the lead's
  counterexample, both wedge tests, tunnel, boundary pit, instance/prototype
  edit invalidation and the dense-cuboid controls.
- `cargo test -p matterweave-detail` → 122 passed, 0 failed across 15 targets.
- `cargo clippy -p matterweave-detail --all-targets -- -D warnings` → clean.
- `python3 tools/check_docs.py` → PASS.

New tests: `joint_closure_holds_source_on_every_axis_and_on_negative_coordinates`
(channel along each axis, block origins `[0, 0, 0]`, `[-16, -16, -16]`,
`[-7, 3, -21]`, cross-section start forced onto an odd global coordinate so it
always straddles) and `stepped_wedge_still_coarsens_on_negative_coordinates`.

### Corrections made during this round

- An intermediate variant of my own test demanded `Source` for a channel whose
  cross-section is coarse *aligned* at factor 2. That expectation was wrong,
  not the guard: such coarse cells are entirely air, are never filled, and the
  channel survives `Half` untouched while the factor-4 fill would erase it.
  Pinned instead by
  `coarse_aligned_two_by_two_channel_coarsens_exactly_as_far_as_it_survives`
  (asserts `Half` and that the channel cells are still `AIR`).
- `site_visits` renamed to `window_site_samples` and documented honestly: it
  counts occupancy-build samples (`analyzed_cells * (factor + 2)^3`), not
  flood-fill or array visits; total array work is a small constant multiple.
- Doc claim about surfaces narrowed to smooth steps and slopes; thin
  concavities, 1-cell crevices and rough organic surfaces remain conservative.
  No general topology-preservation theorem is claimed; the documented gaps are
  skipped fully occupied footprints and bypasses beyond the one-cell halo.

### Cost after the change

Identical analyzed cells and samples to round 4 (terrain 1488/535 cells,
95232/115560 samples at factor 2/4; flora rows unchanged),
`budget_exhausted_cells == 0` everywhere. Sequential evaluation only raised
some loss values, e.g. `parasol_mushroom` 0.375 → 0.625 at factor 2.

Device/Android evidence: NOT RUN. Host `cargo` results only.
