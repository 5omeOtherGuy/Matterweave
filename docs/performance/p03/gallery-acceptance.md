# P03 gallery acceptance and integration handoff

Status: acceptance procedure. No visual or phone result is implied. Native rendering,
physics integration and phone tests are lead-owned; this document defines what those
checks must record.

## What problem this solves

The detail foundation is an isolated source/mesh adapter. This is the procedure that
decides whether it can be integrated, and what evidence an acceptance decision needs.

## How it works

### Prototype inspection

For each accepted mushroom, record seed, generator version, prototype ID, voxel scale,
occupied cells, local metre bounds, source hash, mesh triangles and material palette.
Capture the same geometry from front, side and below, plus a solid silhouette. Frame the
whole specimen with a small margin and include a close crop of the underside. Use a
neutral background and consistent illumination so material colors cannot hide missing
geometry. Record the camera pose and projection with each output.

The initial parasol must show a crown with a shaped profile, a readable rim and
underside, radial gills, a distinct stipe and a connected cap/stem. Check that gills are
actual source/derived geometry, not only stripes painted on a closed flat disc. Check the
cap silhouette from both front and side. A thick undifferentiated dome, floating cap or
noisy disconnected voxels fails regardless of cell count.

Host mesh captures support geometry inspection. They cannot pass the required native
near/middle/distant material, shadow, thin-feature and temporal checks. The native
integrator records actual viewing distances and approach/retreat paths, checks for holes,
winding, shimmer, popping and contact artifacts, and retains source detail.

### Representative tile

Inspect the entire 16 m footprint, then the terrain bank and mushroom ground contacts.
Verify uneven but coherent relief, a meaningful water region and clear collision policy,
nonfloating flora and an accessible route. Do not expand to the full map until tile cost
and the prototype interface have been accepted. Report stored voxel payload and derived
mesh allocation separately; instance-expanded counts describe content reuse, not
duplicated resident memory or unique occupied union volume.

### Native adapter checklist

- Use metre-space meshes/bounds consistently with source-query transforms. Preserve
  existing world/save/generator/material IDs and unit-scale physics.
- Cache prototype meshes by stable ID and source revision; transforms and renderer
  recreation must invalidate the appropriate derived state. No per-frame remeshing.
- Keep static collision and decorative queries explicit; visual LOD cannot remove required
  physical support or modify authoritative cells.
- Record actual peak host/device memory, upload cost and frame distributions on the tile
  before selecting expansion/instancing/LOD changes. Targets are not evidence.
- Verify edit/save/reload and negative/transformed coordinates through the real app, then
  capture phone front/side/underside and near/far views.

## What was verified

Nothing in this procedure has been executed for the native renderer. Until the checks
above are run, these integration checks remain **NOT RUN**.

## Limits and what is open

The procedure covers one fixture prototype, one 16 m tile and a sparse gallery. It cannot
establish dense vegetation, full-map performance, temporal stability or device cost.
