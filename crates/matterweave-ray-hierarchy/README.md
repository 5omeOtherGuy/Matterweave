# matterweave-ray-hierarchy

Bounded **experiment** for [issue 42](https://github.com/5omeOtherGuy/Matterweave/issues/42):
hierarchical voxel ray traversal with packed block occupancy, compared against the
retained dense reference.

Not a production renderer path, not a GPU implementation, not a performance result. See
[the engineering log](../../docs/performance/logs/ray-hierarchy-experiment.md) for the
measured CPU comparison, the prior-art assessment, the disclosed limits and the
integration handoff.

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
```

The differential corpora print their counts with `--nocapture`; the log records the
observed numbers, including zero measured distance deviation from `World::raycast` and
the three tied-plane cell resolutions found in the seeded corpus.
