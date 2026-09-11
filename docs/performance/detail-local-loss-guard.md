# Local interior-loss guard for automatic detail selection

Scope: `matterweave-detail` only. Companion log:
[engine-03-detail.md](logs/engine-03-detail.md). No visual-acceptance or
device-performance claim is made here; on-device approach/retreat/zoom review
remains lead-owned.

## What problem this solves

Any-occupied coarsening fills a narrow void whenever the surrounding solid
occupies the same coarse cell. The global dilation bias
(`1 - occupied / (coarse_cells * factor^3)`) spreads that fill over the whole
prototype, so a small deep opening in a large solid is a negligible fraction
and coarsens away. Measured at 6.25 cm cells (fine scale):

| prototype (16-cell unless noted) | factor | dilation | worst interior loss |
| --- | --- | --- | --- |
| dense solid 8 / 16 | 2, 4 | 0.0000 | 0.0000 |
| 1x1 through-tunnel | 2 | 0.0039 | 0.2500 |
| 1x1 through-tunnel | 4 | 0.0039 | 0.0625 |
| 12-cell blind pinhole | 2 | 0.0029 | 0.2500 |
| 12-cell blind pinhole | 4 | 0.0029 | 0.0625 |
| 2-cell face pit, 13-cell solid | 2 | 0.2001 | 0.5000 |
| 2-cell face pit, 13-cell solid | 4 | 0.4641 (bias blocks Quarter) | 0.1250 |
| isolated 1x1x24 stem | 2 / 4 | 0.7500 / 0.9375 | 0.0000 |
| isolated 16x32x1 sheet | 2 / 4 | 0.5000 / 0.7500 | 0.0000 |

Default bias is 0.45: tunnels pass it, thin stock fails it. Neither heuristic
alone resolves both measured cases; together they resolve the fixtures tested
in this document — a claim scoped to those fixtures, not to untested shapes.

## How it works

### Per-coarse-cell loss

Per prototype revision, per factor `f` in `{2, 4}`, the digest censuses coarse
occupancy and counts occupied source cells per coarse cell (`BTreeMap`, entries
bounded by the occupied-cell count). It records:

- `local_loss_fraction = max((expected - count) / expected)` over occupied
  coarse cells, where `expected` is the footprint size *clipped to the
  occupied cell bounds*. Clipping keeps unaligned dense cuboids at exactly
  `0.0` (boundary partial cells are fully occupied in-bounds) while a notch
  or tunnel mouth inside a boundary cell reads `> 0`. An earlier version that
  skipped non-fully-inside footprints missed exactly that boundary case and
  was replaced.

`choose_lod` breaks out of the coarsening walk when
`local_loss_fraction > max_local_loss_fraction`, exactly like the dilation
bias. `max_local_loss_fraction` defaults to `0.0` and validates as finite within
`0.0..=1.0`. The threshold is applied to the *finest* coarse level first and
the walk breaks there, so loss being non-monotone in coarseness (a tunnel
reads `0.25` at Half and `0.0625` at Quarter) means an intermediate threshold
between those values reaches `Source`, not `Quarter`.

### Local topology gate

Clipping alone is a *global* clip against the prototype's occupied bounds, so
it only neutralizes the outer box: every partially filled coarse cell on a
sloped, stepped or curved surface still read `> 0` and the `0.0` default pinned
essentially all non-cuboid content at `Source`. Partially filled cells are now
gated by a bounded digital-topology test before they contribute loss.

Per partially filled occupied coarse cell, on a `(factor + 2)^3` window (the
footprint plus a one-cell halo; 64 sites at factor 2, 216 at factor 4, fixed
stack scratch, no heap, no flood outside the window):

- **Channel closure**: label 6-connected air before and after filling the
  footprint. If two halo air sites connected before are separated after, the
  fill closes a passage (through-tunnels and channels wider than one cell).
- **Cavity or pit**: an air site inside the footprint that is unreachable from
  the halo (enclosed void), or that has at least four solid face neighbours
  (pinhole, blind pit, slot) — features that through-connectivity cannot see.

Only cells failing one of those contribute `(expected - count) / expected`.
A smooth stepped or sloped surface has one open air region touching the halo
on many sides and at most three solid face neighbours per air site, so it
reads `0.0` and coarsens. That claim is about smooth steps and slopes only:
thin concavities, 1-cell crevices and rough organic surfaces still trip the
pit rule and stay conservative (see the measured terrain/flora rows below).
The test is the block generalization of the simple-point criterion used in 3D
thinning (Bertrand/Malandain); no Rust crate exposes that test outside a full
meshing/skeletonization engine, so it is ~60 lines here rather than a
dependency. It is a bounded per-window check, **not** a proof of global
topology preservation, and none is claimed.

### Sequential evaluation

Per-cell independence against the source is not sufficient. A 2x2 channel
whose cross-section straddles coarse boundaries survives every *single* fill
through a bypass in a neighbouring coarse cell, while the fills together erase
it — the counterexample in
`adjacent_coarse_fills_cannot_jointly_close_a_two_by_two_tunnel`.

Cells are therefore evaluated as a sequence in the deterministic lexicographic
`BTreeMap` key order. The "before" occupancy of a cell virtually includes the
footprint of every occupied coarse cell ordered before it (membership in the
same `counts` map, `div_euclid` bucketing, so negative coordinates bucket
exactly as the digest does); "after" adds the current footprint. The
accumulated end state is the full coarse fill, so the checks decompose the
whole transformation into steps rather than testing each cell against the
untouched source. No extra state, no second pass, no source mutation: the
virtual fill exists only as a predicate over `counts` while sampling a window.

Known gaps, deliberately not papered over:

- Fully occupied (clipped) footprints are still skipped, so the solid they add
  *outside* the occupied bounds is included in later cells' accumulated state
  but never itself analyzed. Analyzing them would report exterior air splits
  that reconnect just outside the window.
- The accumulated state is only visible inside the one-cell halo, so a bypass
  farther than one coarse cell away is not seen at that step.
- A channel that is exactly coarse aligned has *empty* coarse cells, which are
  never filled: it survives that level untouched and correctly coarsens
  (`coarse_aligned_two_by_two_channel_coarsens_exactly_as_far_as_it_survives`
  pins Half for a factor-2-aligned 2x2 channel that the factor-4 fill would
  erase).

Analysis is bounded by `LOCAL_TOPOLOGY_CELL_BUDGET = 8192` analyzed cells per
(revision, factor). Past the budget a partial cell keeps the pre-topology
conservative verdict, which can only hold a finer level, never select an
unsafe one.

### Cost and allocation (not device claims)

- Digest work per source revision: one pass over occupied cells (unchanged
  shape; counts replace presence sets, adding a 4-byte count payload per
  coarse entry plus the map's node overhead and padding under the same
  entry-count bound), plus one constant-time footprint/bounds check per
  occupied coarse cell per factor. No per-frame census: `select_lods` reads
  the cached table.
- Allocation is entry-count bounded (occupied-cell count, within
  `MAX_VOLUME_CELLS`); that bound covers entry payloads and node overhead
  alike, but it is not a byte-exact budget — node layout and padding are the
  allocator's.
- `prepare_batches`, mesh cache behavior, and all collision/query paths are
  unchanged: collision still reads the authoritative source, never the guard.

## What was verified

Host only. Measured on representative prototypes (unit test
`scene::local_topology_cost`), no fixture reaches the budget:

| prototype | occupied | partial cells @2 / @4 | window site samples @2 / @4 | fallback |
| --- | --- | --- | --- | --- |
| `terrain_detail_tile` | 25076 | 1488 / 535 | 95232 / 115560 | 0 |
| `parasol_mushroom` | 938 | 152 / 44 | 9728 / 9504 | 0 |
| `funnel_mushroom` | 1106 | 192 / 59 | 12288 / 12744 | 0 |
| `fan_frond` | 422 | 136 / 49 | 8704 / 10584 | 0 |
| `reed_cluster` | 297 | 108 / 29 | 6912 / 6264 | 0 |

`window_site_samples` counts *occupancy-build samples only*
(`analyzed_cells * (factor + 2)^3`): one source read per site plus at most one
`counts` lookup for an air site. It is not a count of flood-fill steps or
array accesses — the two labelling passes and the neighbour scan revisit the
same window, so total array work is a small constant multiple of the figure.
Sequential evaluation added no cells and no samples (identical counts before
and after); it only raised some loss values, e.g. `parasol_mushroom`
from 0.375 to 0.625 at factor 2. Worst case is `8192 * 216` samples per
factor, once per source revision; `select_lods` still reads only the cached
digest, and the source is never modified.

Behavioral evidence: `crates/matterweave-detail/tests/local_loss.rs` (9 tests
when recorded; the file has since grown to 17). Tunnel and boundary face-pit
held at `Source` far away under perspective and orthographic zoom, each with a
relaxed-guard control that recovers coarsening; unaligned odd-edge dense cuboids
(positive and negative coordinates) still reach `Quarter` while the thin sheet
next to them stays held; adjacent touching instances both reach `Quarter` with a
collidable seam; dense block spans `Quarter`-far/`Source`-near across both
projections; selection + preparation leave revision, snapshot and counts
bit-identical. Full crate: 113 passed / 14 suites; strict Clippy clean.

## Limits and open work

- Enclosed voids trip the guard exactly like through-tunnels; the guard holds
  the whole prototype one level finer (or at `Source`) rather than repairing
  the opening. Callers that can tolerate a specific opening should raise
  `max_local_loss_fraction` for that prototype's config, or cap `max_lod`.
- Thin isolated stock has a fully occupied clipped footprint, so its safety
  still rests on the global dilation bias; relaxing `max_dilation_fraction`
  toward `1.0` re-opens thin-feature loss even with this guard at default.
- Worst-cell sensitivity cuts both ways: one stray interior air cell (e.g. a
  deliberate 1-cell mortise) holds the prototype finer. That is the intended
  trade until a per-opening allowlist exists.
- **Occupancy only.** The guard reads occupancy, not material. A channel or
  pocket filled with `WATER` (or any non-`AIR` material) has no partially
  filled coarse cell, so it is invisible to the guard and `coarsen`'s majority
  vote replaces it with the host material at `Half`/`Quarter`. Pinned by
  `material_filled_channel_is_outside_the_occupancy_guard`. Authoritative
  source, sampling and collision are unaffected. Extending the guard to
  material heterogeneity is open work; a naive version would hold every
  multi-material flora prototype at `Source`.
- The topology gate is still conservative for rough organic surfaces: a
  measured `terrain_detail_tile` and the flora prototypes still read a high
  loss (their surfaces contain genuine 1-cell pits/crevices that trip the
  four-solid-neighbour rule), so they stay at `Source` under the default.
  Smooth stepped/sloped surfaces are the class the topology gate unblocked.
- Negative coordinates are handled (`div_euclid` footprints); the footprint
  products cannot overflow (`|cell| <= 2^16`, `factor <= 4`).
