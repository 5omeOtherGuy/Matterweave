# Full-image renderer comparison reference

The native `renderer_comparison` example executes actual Vulkan surface raster,
authoritative voxel traversal and a two-subpass hybrid sharing color/depth.
It extends the [single-ray reference](ray-reference.md); it does not select a
primary renderer or close M2. The [device manifest](../evidence/2026-09-08-renderer-comparison.json)
retains exact source, binary hash, settings and complete phone output.

## Implemented checks

Four fixtures cover thin plates, negative chunk boundaries, a close perspective
floor and an orthographic diagonal wall. Two fixtures also change material.
Each runs in split mode (disjoint voxel subsets with real mutual occlusion) and
matched mode (complete world in each path): 12 runs total. All paths share camera,
palette, sun, ambient, fog, near/far and 128×128 resolution. No shadows or GI.

Matched mode requires zero color/depth mismatches, including edge pixels, within
3/255 per-channel color and 0.001 normalized depth tolerance. Edge classification
is diagnostic only. Split hybrid must equal the nearer-depth composite of the
individual paths. A grid of 256 CPU rays checks each image, with explicitly
reported boundary/grazing/inside-start exclusions. Both paths must cover pixels;
an edit must change the image and invalidate a pack of the same world. Nonfinite
depth is a failure (an actual failing regression preceded the correction).

The phone and llvmpipe host each pass all 12 runs; all six matched runs have zero
mismatches within tolerance and all hybrid composites agree. Host synchronization
validation reports no errors. Android validation layers were unavailable, so that
check is **not run** on Android. The captured matched images were also inspected.

## Cost interpretation and remaining work

The recorded phone pack times are 0.003–0.013 ms and mesh/serialization times
0.004–0.029 ms. Combined resource/pipeline setup takes 13.831–45.566 ms.
Individual draw/submit/wait/readback calls take 0.336–1.202 ms across paths.
These are single-run wall times for tiny fixtures, not isolated GPU time or
frame-time distributions. Setup currently creates both path resources together;
it cannot assign total cost to a primary renderer. No memory/thermal efficiency
conclusion follows. Matched hybrid draws duplicate geometry as an equivalence
control; only split mode tests partitioned rendering.

M2 still needs representative dense/fine-detail workloads, comparable production
quality, separately attributable preparation/upload/residency costs, repeated
mobile measurements and ADR-0005/0006/0007 conclusions. Shader boundary tolerances
and clipping remain as described in the ray reference. The current edit fixtures
change materials; geometry edits and temporal camera paths need broader coverage.

## Reproduce

```sh
MATTERWEAVE_COMPARISON_OUT=/mnt/bench/matterweave-dev/comparison \
VK_LAYER_ENABLES=VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT \
cargo run --locked -p matterweave-render --example renderer_comparison
```

Use the pinned Android cross-linker documented in [Development](../DEVELOPMENT.md)
to build the same example for `aarch64-linux-android`, push it into a project-named
`/data/local/tmp` path, and set `MATTERWEAVE_COMPARISON_OUT` to a disposable path.
Exit 0 requires all gates; 1 indicates a failure; 2 is unavailable Vulkan.
Output contains 72 PPM/color and PGM/depth images plus a text summary. Preserve
the report as well: the summary alone does not contain timings or oracle counts.
Large evidence currently remains under `/mnt/bench/matterweave-dev/performance/engine-03/`;
its durable delivery archive reference is pending, explicitly recorded in the manifest.
