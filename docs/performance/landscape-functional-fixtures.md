# Landscape functional fixtures for `renderer_comparison`

Opt-in functional fixtures for the existing native `renderer_comparison` example.
They exercise the already working raster, ray and hybrid paths, the CPU
`World::raycast` oracle, the edit gate and the orthographic camera on larger
scenes. They do not add a renderer, select a primary path, or close M2. See
[renderer-comparison.md](renderer-comparison.md) for the base harness.

## Why these fixtures

The default suite is 16 runs of small scenes. Two untested interpretations
matter for the landscape direction:

1. **Thin near detail against a distant occluding silhouette.** One-cell
   vegetation in front of a far ridge stresses near-depth ordering of thin
   surfaces where rasterization and analytic traversal can disagree.
2. **Negative-coordinate terrain with an opening, seen orthographically.**
   Editable solid terrain near chunk boundaries with a doorway and an
   orthographic camera.

The fixtures are a **local fine-scale interpretation**: 1-unit voxel cells at
128x128. They are not production microvoxel integration, not a fastest-path
claim, and not Android measurements.

## How to run

```sh
CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/microvoxel-roadmap/target-fixtures \
MATTERWEAVE_COMPARISON_OUT=<output dir, e.g. /mnt/bench/.../landscape-artifacts-fixed> \
cargo run --locked -p matterweave-render --example renderer_comparison -- --landscape
```

- Default invocation (no flag) is unchanged: exactly the original 16-run
  gate, same output directory default `target/engine-03-comparison`, same
  exit codes.
- `--landscape` appends the two landscape fixtures: 8 extra runs
  (split/matched, unedited/edited) for 24 total.
- All other arguments are ignored, as before.

## Fixtures

### `vegetation-vs-distant-ridge`

- Distant occluding terrain: solid ridge at `z = 40..42`, `x = -24..24`,
  `y = -20..top(x)`, with a stepped top profile 6..12 and rock/sand materials.
- Near thin detail: one-cell stems at `z = 3`, `x in {-3,-1,1,3}`, heights
  3..5, each with one-cell-thick fronds at `z = 3..4`. No vegetation cell lies
  on the split column, and the 4-unit depth range keeps the detail near.
- Camera: perspective `eye [0,4,-6] -> target [0,3,30]`, vfov 0.9 rad.
  The ridge sits at about 49 units from the eye, inside `FAR = 90`.
- Edit: remove stem cell `[-3,1,3]` (a gap through the near stem).
- Near-detail gate: the oracle must report at least one hit inside
  `[-5,0,3]..=[5,5,4]` (observed 35-84); terrain hits alone cannot satisfy it.
- Split `x = 0` keeps near stems and ridge geometry on both sides.

### `negative-ortho-opening-edit`

- Negative-coordinate terrain: ground slab `x -44..-16`, `z -48..-2`,
  `y -6..-3`; front wall `z -20..-19`, `y -2..8` with a 3-wide, 6-high opening
  at `x -31..-29`, `y -2..3`; back wall `z -46..-45`, `y -2..8`; boulders and a
  pillar stand on the ground.
- Camera: orthographic `eye [-30,14,6] -> target [-30,-3,-30]`, half-extent 16
  (128 px / 32 world units = 4 px per cell).
- Edit: add cell `[-30,0,-19]`, blocking the lower middle of the opening.
  The changed hybrid patch is 24 px at `x 64..67, y 68..73`, fully inside the
  frame, so the edit is neither occluded nor clipped.
- Split `x = -36` keeps wall/ground geometry on both sides; the edit is not on
  the dropped column.

## Bounds confirmed from source

`RayVolume::pack` (crates/matterweave-render/src/ray_reference.rs) rejects
dimensions outside `1..=128` per axis and more than `MAX_CELLS = 64^3 =
262144` cells, and requires every AABB endpoint within +/-8192. `bounds()`
in the example silently clamps each axis to 128, so the unit test also proves
no cell is cropped.

| Fixture | AABB dims (x,y,z) | Cells in pack | Within bounds |
| --- | --- | --- | --- |
| vegetation-vs-distant-ridge | 51 x 35 x 42 | 74970 | yes |
| negative-ortho-opening-edit | 31 x 17 x 49 | 25823 | yes |

Unit tests run without a GPU:
`cargo test --locked -p matterweave-render --example renderer_comparison`
covers `landscape_fixtures_fit_declared_ray_volume_bounds` (axis/cell bounds,
no silent crop, surface-changing edit, near detail present on both split sides,
edit not on the split column) and `default_gate_keeps_its_fixture_set`.

## What was verified

Host: llvmpipe / lavapipe (Mesa 25.2.8, LLVM 20.1.2, Vulkan 1.4.318),
validation layer `VK_LAYER_KHRONOS_validation` active, errors are failures.
Base revision `7dace54` plus the working-tree changes in
`crates/matterweave-render/examples/renderer_comparison.rs`.

- Default gate: `16 fixture runs, 0 failures`, exit 0, validation no errors.
  (Raw log: `/mnt/bench/matterweave-dev/microvoxel-roadmap/fixtures/default-run-final.log`.)
- `--landscape`: `24 fixture runs, 0 failures`, exit 0, validation no errors.
  (Raw log: `/mnt/bench/matterweave-dev/microvoxel-roadmap/fixtures/landscape-run-final.log`.)

Per fixture (split, then matched; unedited/edited in parentheses):

| Fixture / mode | Coverage raster / ray / hybrid | Matched ray-vs-raster | Oracle hits (detail hits) | Edit changed |
| --- | --- | --- | --- | --- |
| vegetation split (un) | 5251 / 5664 / 10915 | dispatch content-diff only | 84 (37) | - |
| vegetation split (edit) | 5251 / 5664 / 10915 | dispatch content-diff only | 84 (35) | 245 px |
| vegetation matched (un) | 11209 / 11209 / 11209 | 0 unexplained | 167 (84) | - |
| vegetation matched (edit) | 11209 / 11209 / 11209 | 0 unexplained | 167 (82) | 245 px |
| ortho split (un) | 9920 / 3968 / 13888 | dispatch content-diff only | 64 (0 required) | - |
| ortho split (edit) | 9920 / 3968 / 13888 | dispatch content-diff only | 64 (0 required) | 24 px |
| ortho matched (un) | 14384 / 14384 / 14384 | 0 unexplained | 240 | - |
| ortho matched (edit) | 14384 / 14384 / 14384 | 0 unexplained | 240 | 24 px |

Every run additionally required nonzero coverage from both paths, a zero-diff
hybrid-vs-min-depth composite, zero pack-oracle mismatches, the marker to
project and be covered, the edit to change the hybrid image and invalidate the
pack, and zero validation errors. Costs are printed as single-run wall times
(pack, mesh, combined resource/pipeline setup, draw+readback per path); they
are not isolated GPU time and no performance conclusion is drawn.

## Preserved boundary failure and candidate explanation

The first `--landscape` run failed fixture 2 matched on exactly 116 pixels in
image row 0 (`x 8..123`), for both the unedited and edited run; the raster had
rgb (46,54,54) at depth 0.55442 while the ray had background. The log, summary
and raster/ray/hybrid PPM/PGM files are preserved unchanged in
`/mnt/bench/matterweave-dev/microvoxel-roadmap/fixtures/landscape-failed-toprow116/`.

Worker geometric explanation (not independently established as the complete renderer root cause): the back wall's top surface was `y = 7` with far edge
`(y=7, z=-46)`, which projected to view-space `y = 15.8746`, while the row-0
pixel centre is `y = 15.875` (0.0016 px apart). Rasterization resolved that
near-degenerate edge as covered; the pixel-centre ray crossed `y=7` at
`z=-46.0009`, 0.0009 world units behind the wall, and saw background.
This was **not** near/far clipping: the depth maps to exactly 50.0 units, well
inside `NEAR = 0.25` and `FAR = 90`. The declared gate only excuses a
difference when CPU rays within 0.001 px reach two different solid faces and
reproduce both images, so background was correctly not excused.

No threshold was loosened and no check was skipped. The framing was changed so
the row-0 pixel centre hits the back wall's front face at ~`y=7.945`, with
more than 1 unit of margin below the wall's top edge, and the wall's top
surface now projects outside the frame; row 0 has 0 raster/ray differences.

Lead disposition: the adjusted fixture passes; the original coverage mismatch remains
a retained regression case, not a proven renderer fix. Do not generalize the suggested
framing workaround into permission to avoid silhouette tests.
