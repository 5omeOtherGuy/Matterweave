# Local interior-loss guard for automatic detail selection

Scope: `matterweave-detail` only. Companion log:
[engine-03-detail.md](logs/engine-03-detail.md). No visual-acceptance or
device-performance claim is made here; on-device approach/retreat/zoom review
remains lead-owned.

## Problem

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
| isolated 1x1x24 stem | 2 / 4 | 0.7500 / 0.9375 | 0.0000 |
| isolated 16x32x1 sheet | 2 / 4 | 0.5000 / 0.7500 | 0.0000 |

Default bias is 0.45: tunnels pass it, thin stock fails it. Neither heuristic
alone covers both; together they do.

## Algorithm

Per prototype revision, per factor `f` in `{2, 4}`, the digest already
censused coarse occupancy; it now counts occupied source cells per coarse cell
(`BTreeMap`, entries bounded by the occupied-cell count) and records:

- `local_loss_fraction = max(1 - count / f^3)` over occupied coarse cells
  whose source footprint `[c*f, c*f+f-1]` lies fully inside the prototype's
  occupied cell bounds, else `0.0`.

`choose_lod` breaks out of the coarsening walk when
`local_loss_fraction > max_local_loss_fraction`, exactly like the dilation
bias. `max_local_loss_fraction` defaults to `0.0` (any interior heterogeneity
holds the finer level) and validates as finite within `0.0..=1.0`. Boundary
coarse cells are excluded by construction, so surface steps on dense solids
(which read exactly `0.0`) never trip it.

## Cost quantities (not device claims)

- Digest work per source revision: one pass over occupied cells (unchanged
  shape; counts replace presence sets, +4 bytes per coarse entry worst case),
  plus one constant-time footprint/bounds check per occupied coarse cell per
  factor. No per-frame census: `select_lods` reads the cached table.
- Allocation is bounded before use by the existing volume cell budget
  (`MAX_VOLUME_CELLS` via `MAX_VOLUME_CHUNKS`); entries cannot exceed the
  occupied-cell count.
- `prepare_batches`, mesh cache behavior, and all collision/query paths are
  unchanged: collision still reads the authoritative source, never the guard.

## Conservative limits

- Enclosed voids trip the guard exactly like through-tunnels; the guard holds
  the whole prototype one level finer (or at `Source`) rather than repairing
  the opening. Callers that can tolerate a specific opening should raise
  `max_local_loss_fraction` for that prototype's config, or cap `max_lod`.
- Thin isolated stock has no interior footprint, so its safety still rests on
  the global dilation bias; relaxing `max_dilation_fraction` toward `1.0`
  re-opens thin-feature loss even with this guard at default.
- Worst-cell sensitivity cuts both ways: one stray interior air cell (e.g. a
  deliberate 1-cell mortise) holds the prototype finer. That is the intended
  trade until a per-opening allowlist exists.
- Negative coordinates are handled (`div_euclid` footprints); the footprint
  products cannot overflow (`|cell| <= 2^16`, `factor <= 4`).

## Behavioral evidence

`crates/matterweave-detail/tests/local_loss.rs` (7 tests): tunnel held at
`Source` far away under perspective and orthographic zoom with a relaxed-guard
control that recovers coarsening; stem/sheet held while the dense block next
to them reaches `Quarter`; adjacent touching instances both reach `Quarter`
with a collidable seam; dense block spans `Quarter`-far/`Source`-near across
both projections; selection + preparation leave revision, snapshot and counts
bit-identical. Full crate: 111 passed / 14 suites; strict Clippy clean.
