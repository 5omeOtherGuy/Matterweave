# Engineering log w_0e4cfd00

Worker: `deepseek-flash-go` · outcome: and# Worker brief — vegetation must have structure, not be cube props · run: `/home/someotherguy/.pi/agent/subagent-dashboard/runs/w_0e4cfd00`

- **Actions Taken:** Redesigned the landscape flora prototypes in `matterweave-detail`
  (grass clumps of 1-voxel blades at the 6.25 cm fine scale, 8..=20 cells tall on a
  one-cell pad plate; flowers as 2..=5 thin stems with blooms; broadleaf and conifer
  crowns built from walked limbs and fronds instead of a solid leaf ellipsoid).
  Rebalanced placement in `matterweave-core` (denser grassy biomes, a second
  ground-cover slot per metre column, coastal palms, two density bands to 28 m and
  40 m). Added tree-specific LOD bands in the explorer runtime, a fixed-camera
  override and a shadow-only flora capture mode, a blade bend test, and the
  capture-protocol notes in `docs/DEVELOPMENT.md`.
- **Issues & Friction:** The spawn camera faces open ocean, so 0 of 157 trees can
  project into it at any distance; the tree cause is placement relative to the fixed
  capture, which is why the tree evidence uses a named land-facing camera. The
  3x vertical-run criterion is not reachable while blades stay inside the brief's
  8..20 fine-voxel bound, because the previous prototypes were already 4..6 fine
  voxels tall. The host smoke runs on llvmpipe: the denser field is ~4x the
  triangles, so the software rasteriser's render time regresses 2.3x even though the
  printed frame time (clamped at 100 ms) reads +10%.
- **Decisions & Rationale:** Kept the blade lean and tip material (both are asked
  for) rather than trading them for the host frame number; dropped the far band to
  a sparse eighth ending exactly at the stated sub-pixel crossover instead. Used a
  `shadow` reference frame for the coverage metric so vegetation pixels can be
  separated from the shadows they cast.
- **Solutions Applied:** `LANDSCAPE_FLORA_VERSION` 3, `LANDSCAPE_GENERATOR_VERSION`
  2 and a new placement fingerprint; `MATTERWEAVE_LANDSCAPE_EYE` and
  `MATTERWEAVE_LANDSCAPE_FLORA=off|shadow` measurement modes; `MAX_PLANNED_SITES`
  12000; plan time worst < 3 ms with `over 0`; drawn instances 4958 -> 6521.
- **Insights:** The pixel metrics a coverage change is judged by need a reference
  frame with the same shadows but no vegetation; a colour classifier cannot separate
  a shaded blade from a terrain step face in this palette. On a software rasteriser
  the flora field is vertex/raster bound, so the host smoke number tracks triangle
  count, not instance count.
