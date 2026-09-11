# Automatic view-dependent detail (LOD) selection

Scope: `crates/matterweave-detail` engine-side automatic level-of-detail
selection and derived-mesh preparation for `DetailScene`. Implements part of
[R08](../REQUIREMENTS.md) under [ADR-0007](../adr/0007-virtualized-detail.md).
Native renderer integration and on-device acceptance are lead-owned; this
document covers the host-verified engine slice only.

## What problem this solves

A `DetailScene` stores prototypes as coarse volumes at several levels, and a
renderer adapter must pick a level per instance from the camera and obtain the
derived geometry to draw. Nothing selected levels or produced uploaded-ready
batches.

## How it works

`DetailScene` selects a per-instance LOD from the existing `Source`/`Half`/`Quarter`
representations and prepares the derived meshes an adapter uploads directly:

- `DetailScene::select_lods(&Camera, &LodConfig) -> Vec<InstanceLod>` — pure
  selection with hysteresis; no meshes built, no authoritative data touched.
- `DetailScene::prepare_batches(&Camera, &LodConfig) -> PreparedFrame` — selection
  plus synchronous, budgeted, lazy mesh realization grouped into per-`(prototype,
  lod)` `MeshBatch`es with instance transforms. `Source` is the retained fallback.
- `DetailScene::cached_prototype_mesh(id, lod) -> Option<&Mesh>` — immutable access
  to already-built geometry for upload, no `&mut` and no build.

`Camera` carries `eye_m`, a nonzero `forward_m`, `viewport_height_px`, `near_m` and a
`Projection` (`Perspective { vertical_fov_rad }` or `Orthographic { view_height_m }`).
Zoom is the FOV (perspective) or the view height (orthographic).

`LodConfig` exposes `error_budget_px`, `hysteresis`, a `max_lod` quality cap
(`Lod::Source` or `LodConfig::disabled()` turns LOD off), a `max_dilation_fraction`
thin-feature bias and an optional `max_coarse_builds` work cap. Numeric input
validation is described in [LOD numeric input validation](detail-numeric.md), and
the local interior-loss guard in [local interior-loss guard](detail-local-loss-guard.md).

### Error model: estimates, not guarantees

Any-occupied coarsening has no cheap tight geometric bound, so the engine reports
**estimates**, never guarantees. An earlier "conservative bound" framing was
rejected on review and does not describe this code.

- `error_estimate_m = scale * factor` is the coarse cell size: a heuristic error
  magnitude, **not** a Hausdorff bound. Filling a long narrow cavity deletes
  interior faces arbitrarily far from the remaining surface, and a lone diagonal
  protrusion moves by up to the cell diagonal, already exceeding one edge.
- `dilation_fraction` is the whole-prototype fraction of coarse solid newly filled
  by expansion. It **biases** thin/perforated prototypes (fronds, stems, sheets)
  toward finer levels, which is why the visually rejected coarse flora stay at
  `Source`. It is global: a small deep opening inside a large solid contributes a
  negligible fraction and is **not** guaranteed to be preserved. A caller needing a
  specific opening kept must cap the level. This limitation is pinned by
  `tests/lod_correctness.rs::deep_pinhole_opening_is_not_guaranteed_preserved_by_the_global_bias`.
- The pixel figure is a working prioritization estimate, not proof of temporal
  visual quality. Perspective uses the **nearest instance-AABB depth along
  `forward_m`** (clamped to `near_m`), not Euclidean eye distance, which overstates
  depth for off-axis instances.

## What was verified

Host only; no mobile numbers exist.

- Approach/retreat and FOV/orthographic zoom move between `Source`/`Half`/`Quarter`.
- Orthographic selection is depth-invariant; perspective uses view-forward depth
  (off-axis instances at equal forward depth get equal LOD).
- Hysteresis dead-band prevents boundary chatter under rapid reversal.
- Negative coordinates, near-plane/eye-inside and invalid camera/config inputs are
  handled without panics.
- Edits invalidate stale derived LOD meshes and advance `source_version`.
- World collision/queries and source counts are invariant to the camera.
- Work/memory budgets: `max_coarse_builds` and the mesh-cache cap fall back to the
  authoritative `Source` mesh, flagged per instance.

Tests: `tests/lod_selection.rs` (15), `tests/lod_correctness.rs` (3),
`tests/source_version.rs` (contract preserved). Deterministic host example:
`cargo run -p matterweave-detail --example lod_sweep` (LOD counts and mesh bytes
across a camera sweep with stable source counts; no performance claim).

## Reuse rationale (COMPONENT_SELECTION)

Selection math (screen-space error estimate, hysteresis dead-band, nearest-depth
projection) is a handful of `f32`/trig expressions over the crate's existing
`coarsen`/`mesh_local` pipeline. Rust mesh-simplification crates (e.g. meshopt-style
decimation, geometry streaming) target triangle meshes and bring renderer-adjacent
dependencies; none selects among voxel coarse-volume LODs by projected error, and
adopting one would not replace this glue. No dependency crossed the strict criteria,
so no new dependency was added — avoiding unpinned dependency risk. The projection
uses only `std` `f32` methods. This is recorded here rather than as a full ADR
because it is a routine implementation detail under proposed ADR-0007 and accepted
ADR-0014.

## Limits and open work

- Native Vulkan batch upload, draw submission and on-device approach/retreat/zoom
  capture review (R08 acceptance evidence).
- Asynchronous versioned request/publication: the current API is synchronous
  bounded lazy preparation, as permitted for a first slice.
- A genuinely tight geometric error bound and per-opening preservation guarantee.
