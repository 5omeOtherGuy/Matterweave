# Full-image renderer comparison reference

The native `renderer_comparison` example executes actual Vulkan surface raster,
authoritative voxel traversal and a two-subpass hybrid sharing color/depth. It extends the
[single-ray reference](ray-reference.md). **It does not select a primary renderer or close
M2.** The [device manifest](../evidence/2026-09-08-renderer-comparison.json) retains exact
source, binary hash, settings and complete phone output.

## What problem this solves

M2 needs matched evidence about the three candidate rendering paths (raster, ray,
hybrid) at the same visual quality. This fixture compares their actual outputs pixel by
pixel and records how the comparison was run.

## How it works

Five fixtures cover thin plates, negative chunk boundaries, a close perspective floor, an
orthographic diagonal wall and an orthographic opening removal. Edits cover material
change, adding geometry and removing geometry.

Each fixture runs in:

- **split mode** — disjoint voxel subsets with real mutual occlusion; the hybrid must
  equal the nearer-depth composite of the individual paths.
- **matched mode** — the complete world in each path.

That is 16 runs total. All paths share camera, palette, sun, ambient, fog, near/far and
128×128 resolution. There are no shadows or GI.

Matched mode requires zero unexplained color/depth mismatches within 3/255 per-channel
color and 0.001 normalized depth tolerance. A maximum 0.05% of pixels may differ only when
independently traced CPU rays within 0.001 pixel reach two faces and reproduce BOTH
measured colors/depths. Missing coverage and arbitrary colors are never excused. This is a
working fixture threshold, not an owner-approved quality guarantee. The original
coverage/depth-neighbour edge mask stays diagnostic.

A grid of 256 CPU rays checks each image, with explicitly reported boundary, grazing and
inside-start exclusions. Both paths must cover pixels; an edit must change the image and
invalidate a pack of the same world. Nonfinite depth is a failure, and an actual failing
regression preceded that correction.

## What was verified

- At `bfb65ca` the phone and llvmpipe host each pass the initial 12 runs, with zero matched
  differences. The expanded host gate passes 16 runs after explicit handling of one/two
  face-edge pixels in the new removal fixture. Final Android repeat is pending.
- Both actual output colors/depths match independent CPU face candidates; regression tests
  reject unrelated color corruption and coverage loss.
- Host synchronization validation reports no errors. Android validation layers were
  unavailable, so that check is **not run** on Android.
- The captured matched images were inspected.

### Cost interpretation

The recorded phone pack times are 0.003–0.013 ms and mesh/serialization times
0.004–0.029 ms. Combined resource/pipeline setup takes 13.831–45.566 ms. Individual
draw/submit/wait/readback calls take 0.336–1.202 ms across paths. These are single-run wall
times for tiny fixtures, not isolated GPU time or frame-time distributions.

Setup currently creates both path resources together, so it cannot assign total cost to a
primary renderer. No memory or thermal efficiency conclusion follows. Matched hybrid draws
duplicate geometry as an equivalence control; only split mode tests partitioned rendering.

## Limits and what is open

M2 still needs representative dense/fine-detail workloads, comparable production quality,
separately attributable preparation/upload/residency costs, repeated mobile measurements
and ADR-0005/0006/0007 conclusions. Shader boundary tolerances and clipping remain as
described in the ray reference. The current edit fixtures include material change and
voxel addition/removal; temporal camera paths and representative workloads need broader
coverage.

### Reproduce

```sh
MATTERWEAVE_COMPARISON_OUT=/mnt/bench/matterweave-dev/comparison \
VK_LAYER_ENABLES=VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT \
cargo run --locked -p matterweave-render --example renderer_comparison
```

Use the pinned Android cross-linker documented in [Development](../DEVELOPMENT.md) to
build the same example for `aarch64-linux-android`, push it into a project-named
`/data/local/tmp` path, and set `MATTERWEAVE_COMPARISON_OUT` to a disposable path. Exit 0
requires all gates; 1 indicates a failure; 2 is unavailable Vulkan. Output contains 96
PPM/color and PGM/depth images plus a text summary. Preserve the report as well: the
summary alone does not contain timings or oracle counts.

Large evidence currently remains under
`/mnt/bench/matterweave-dev/performance/engine-03/`; its durable delivery archive
reference is pending, explicitly recorded in the manifest.
