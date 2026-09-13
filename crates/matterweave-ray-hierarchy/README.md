# matterweave-ray-hierarchy

Bounded **experiment** for [issue 42](https://github.com/5omeOtherGuy/Matterweave/issues/42):
hierarchical voxel ray traversal with packed block occupancy, compared against the
retained dense reference.

Not a production renderer path and not a performance result. See
[the CPU engineering log](../../docs/performance/logs/ray-hierarchy-experiment.md) for the
measured CPU comparison, the prior-art assessment, the disclosed limits and the
integration handoff, and
[the GPU log](../../docs/performance/logs/ray-hierarchy-gpu.md) for the optional native
Vulkan functional comparison of the `BlockMask` kernel (issue 42). The library itself
stays CPU-only; the GPU candidate is a standalone example that borrows the crate's
snapshots and fixtures.

## What it contains

- `BlockShape` (4x4x4, 4x4x8, 8x8x8 and validated custom shapes, at most 512 bits) and
  `OccupancyGrid`: one occupancy bit per cell in fixed blocks, packed in `u32` words and
  kept beside, never instead of, the dense material words.
- `HierarchyVolume`: a derived snapshot built from an existing
  `matterweave_render::ray_reference::RayVolume` pack, so crop validation, the
  `MAX_CELLS`/`MAX_AXIS`/`+/-8192` bounds and the epoch/revision/seed staleness rule are
  reused rather than duplicated.
- Three traversal modes over one shared entry and step implementation:
  `Reference` (dense per-cell DDA), `BlockMask` (block occupancy decides memory access)
  and `BlockStep` (`BlockMask` plus a coarse step over empty blocks).
- Bounded accounting: `MemoryStats`, `BuildReport`, `PatchReport`, `TraversalStats` and
  the reference's iteration cap.
- `patch_cell`: a bounded derived-snapshot update that marks the snapshot stale for its
  source world. Authoritative voxels stay in `matterweave_core::World`.

## Commands

```sh
cargo test -p matterweave-ray-hierarchy
cargo clippy -p matterweave-ray-hierarchy --all-targets -- -D warnings
cargo fmt -p matterweave-ray-hierarchy -- --check

# Optional native GPU candidate (real Vulkan device required; exit 2 = NOT RUN):
cargo run -p matterweave-ray-hierarchy --example ray_hierarchy_gpu
# deterministic host-software run:
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  cargo run -p matterweave-ray-hierarchy --example ray_hierarchy_gpu
```

The example's classifier unit tests (positive and negative controls) run with
`cargo test -p matterweave-ray-hierarchy --example ray_hierarchy_gpu`.

The differential corpora print their counts with `--nocapture`; the log records the
observed numbers, including zero measured distance deviation from `World::raycast`, the
three tied-plane resolutions and the four crop-boundary ties found in the seeded corpus,
and the strict geometric classifier that separates those from real errors. The review
disposition and re-run evidence are in
[the repair log](../../docs/performance/logs/ray-hierarchy-review-repair.md).
