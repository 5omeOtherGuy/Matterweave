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
