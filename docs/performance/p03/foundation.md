# P03 detail foundation: `matterweave-detail`

Status: working prototype foundation, not a shipped feature and not the showcase. Commit
base `94dc2ca`. Revision A2 applies the lead's verified defect list
([log](logs/opus-detail-a2.md)).

## What problem this solves

The engine needs fine-scale authoritative detail — flora, props, terrain tiles — without
rescaling the existing world, physics, save format or core material IDs. `matterweave-detail`
adds detail volumes, prototypes and instances whose cell size and world transform are
explicit and validated.

Components are reused rather than reimplemented:

| Need | Reused component | Rationale |
| --- | --- | --- |
| Sparse editable storage, negative coordinates, revisions, chunk-seam invalidation | `matterweave_core::World` | Already correct and tested for Euclidean chunk keys, revision monotonicity and neighbour invalidation. Rebuilding it would duplicate behaviour and risk divergence. |
| Greedy surface meshing with cross-chunk face removal | `World::mesh_chunk` / `chunk_halo` (block-mesh 0.2.0) | Deterministic, material-merging greedy quads with a one-voxel halo, so shared faces at chunk seams are already removed. No new mesher was written. |
| GPU-facing vertex layout | `matterweave_core::{Mesh, Vertex}` | Keeps the render adapter seam unchanged. |
| Snapshot encoding | `serde` + `serde_json` (already pinned) | No new dependency. |

New dependencies: none. The crate depends only on `matterweave-core`, `bytemuck`, `serde`
and `serde_json`, all at existing pins.

## How it works

### Conventions

- Local cell `c` at scale `s` occupies local metres `[c*s, (c+1)*s]`; the volume origin is
  the corner of cell `[0,0,0]`. Cells are signed `i32`, bounded by `MAX_CELL_COORD = 65536`
  per axis.
- `Scale::new` accepts finite values in `[0.001, 1.0]` m only. Fixtures use
  `SCALE_FINE_M = 0.0625` (flora) and `SCALE_TILE_M = 0.25` (terrain tile).
- `Transform` = quarter-turn yaw (`Yaw::Deg0/90/180/270`) about local +Y, then a
  translation in world metres bounded by `MAX_SCENE_TRANSLATION_M`. The translation is an
  arbitrary finite `f32` per axis, so **fractional-metre placement is supported**; only
  rotation is restricted. That keeps normals axis aligned and bounds computable from
  rotated corners. Everything is `f32`, so no exactness is claimed for arbitrary
  floating-point inputs: a query point exactly on a cell boundary resolves by `f32`
  rounding of the inverse transform. Arbitrary affine transforms (scale, shear,
  non-quarter rotation) are not supported.
- `Transform` fields are public, so a value can be built unvalidated. Every consumer
  (`sample_world_metres`, `mesh_world`, `bounds_world`, `place`) revalidates and returns
  `InvalidTransform` instead of producing results.
- Cell-range checks use signed range membership, never `abs`, so `i32::MIN` is rejected
  rather than panicking in debug or wrapping in release.
- Point-taking queries reject NaN, infinities and out-of-range coordinates with
  `InvalidPoint`; a NaN never resolves to the origin cell.

### API seam

```text
Scale, Transform, Yaw, Bounds, Lod, DetailError
DetailVolume: get, set -> Result<bool>, iter_cells, occupied_cells (O(1)),
              chunk_count, source_bytes, revision, cell_bounds, bounds_local,
              bounds_world -> Result<Option<Bounds>>,
              cell_at_local_metres/sample_local_metres/sample_world_metres -> Result,
              mesh_local/mesh_world -> Result<Mesh>, coarsen -> Result<DetailVolume>,
              snapshot, from_snapshot, from_json_bytes
DetailScene:  add_prototype, prototype, edit_prototype(id, cell, material),
              invalidate(id), place, prototype_mesh(lod) -> Result<&Mesh>, draws,
              sample_world_metres -> Result<Option<..>>, sample_all_world_metres,
              is_collidable_world_metres -> Result<bool>, counts
material::*, material_name, material_color, material_policy
fixtures:     terrain_detail_tile(id, seed) -> Result, parasol_mushroom(id) -> Result,
              gallery_scene(seed) -> Result<DetailScene>, column_top
```

A native/render adapter consumes one cached `Mesh` per (prototype, LOD) in
prototype-local metres, plus `draws()` giving instance id, prototype id and transform.
Meshes are never generated per instance. `Mesh::revision` is always the authoritative
*source* prototype revision, including for coarse LODs.

There is no `&mut DetailVolume` accessor on a scene. Mutation goes through
`edit_prototype`, which preserves the prototype identity and, only when data changes,
drops every cached mesh for that prototype, so a whole-volume swap can no longer serve a
stale mesh at an unchanged revision.

Queries: `sample_world_metres` keeps a documented first-hit-by-instance-id policy for
generic inspection. `is_collidable_world_metres` scans *all* overlapping instances, so a
liquid or decorative instance cannot mask a solid one; `sample_all_world_metres` returns
every overlap.

### Materials and policy

The crate owns a named palette (`detail_soil 10`, `moss_turf 11`, `bank_stone 12`,
`water 13`, `mushroom_stipe 20`, `mushroom_cap 21`, `mushroom_rim 22`, `mushroom_gill 23`).
No existing core material ID is changed or reused. Core's greedy mesher emits its own
colour; `mesh_local` overwrites the colour per greedy quad by re-sampling the owning cell
half a cell inward from the quad centroid — one lookup per quad, and the measured cost is
included in the example timings. Policy: `water` is `Liquid` (authoritative, queryable,
never a collision wall); all other detail materials are `Collision`.
`is_collidable_world_metres` applies the policy. Flora needs no dynamic physics.

### Derived LOD

`coarsen(Lod::Half|Quarter)` produces a *separate* volume at `scale * factor` using a
deterministic majority vote per coarse cell (ties resolve to the lowest material ID).
Votes are a short per-cell association list, not a dense `[u32; 256]` table. The derived
scale is never clamped: if `scale * factor` leaves `[MIN_SCALE_M, MAX_SCALE_M]` the call
returns `InvalidScale` and the caller (including `prototype_mesh`) propagates it, because
clamping silently shrank derived geometry relative to the source. A coarse cell is
occupied when any source cell inside it is occupied, so coarse bounds may conservatively
expand by up to one coarse cell per face; the tests assert source preservation and this
conservative expansion, not bound equality. The source is untouched: cells, revision,
bounds and snapshot are identical before and after coarsening. No seamless LOD transition
and no Nanite-equivalent continuous detail is implemented.

### Budgets

All limits are checked *before* the allocation they guard, and a failure returns `Err`
while leaving prior data and caches untouched — never silent truncation.

| Limit | Value | Guards |
| --- | --- | --- |
| `MAX_VOLUME_CHUNKS` | 1024 (4 MiB) | checked before a `set` allocates a new chunk |
| `MAX_VOLUME_CELLS` | 4,194,304 | snapshot load aggregate |
| `MAX_SCENE_SOURCE_BYTES` | 32 MiB | `add_prototype` and `edit_prototype`, before new chunk allocation |
| `MAX_SCENE_CACHE_BYTES` | 64 MiB | retained vector capacities plus conservative new-output reservation, before building |
| `MAX_MESH_BYTES` | 32 MiB | 24 vertices + 36 indices per occupied cell, before meshing/coarsening |
| `MAX_SNAPSHOT_RUNS` / `MAX_SNAPSHOT_JSON_BYTES` | 2,000,000 / 16 MiB | snapshot load, JSON byte cap checked before serde allocates |
| `MAX_CELL_COORD` | ±65,536 | every cell and point |

These are conservative desktop prototype budgets sized for the present fixtures (tile: 32
chunks, 128 KiB). They are not a phone feasibility claim, and the mesh bound is
deliberately pessimistic, so a dense multi-million-cell volume is refused rather than
meshed. `occupied_cells` and `chunk_count` are maintained incrementally by `set` (O(log n)
map lookups), so generation no longer rescans every chunk per inserted cell.

### Serialization

`VolumeSnapshot` stores X-axis runs `[x, y, z, length, material]` in stable iteration order
(chunk key, then z, y, x), so equal content yields equal bytes. Loading is two-phase:
phase 1 validates every run (length, material range, cell range, axis overflow), aggregates
total length against `occupied_cells` and `MAX_VOLUME_CELLS`, counts distinct chunks
against `MAX_VOLUME_CHUNKS`, and rejects overlapping runs on a sorted copy — independently
of the declared count, so identical or contradictory duplicates whose lengths happen to
sum correctly are refused. Phase 2 writes the validated content; nothing is allocated for
the payload before that. Because runs are chunk-major, global x/y/z ordering is not
required and the loader accepts the crate's own output. `from_json_bytes` enforces
`MAX_SNAPSHOT_JSON_BYTES` before serde parses. Decoding a `VolumeSnapshot` struct from any
other transport is the caller's responsibility to bound. Restoring rebuilds content, not
history: the restored volume starts a fresh revision sequence (documented, tested).

## What was verified

### Gallery scene and measured output (HOST ONLY)

Scene composition lives in `gallery_scene(seed)` (a reusable fixture a native adapter or
test can call), not in the example. Mushroom placements are derived from authoritative
source queries: a candidate tile column is accepted only when the whole foot footprint
(3 fine cells radius, so the centre column plus one neighbour each way) shares one dry
`moss_turf` surface height above the water level, and the instance translation *is* that
derived surface height. No arbitrary vertical offsets are used anywhere. The result is a
deliberately sparse demonstration gallery — one tile and six reused mushrooms — and is
labelled as such in the manifest; it is not a density claim.

`cargo run -p matterweave-detail --release --example detail_gallery` (desktop host, seed
2026) writes to `desktop-01/detail-gallery-a2/`. `manifest.json` holds only reproducible
content; host measurements are separated into `timing.json`, so manifests can be compared
byte-for-byte. Exported vertex and index buffers are explicitly little-endian (`*.le.bin`),
not a native struct cast.

- prototypes 2, instances 7 (1 tile + 6 reused mushroom placements)
- unique stored cells 26,113; expanded occupied cells 30,803 (collision 30,204, liquid
  599) — never conflated
- source payload 163,840 B (32 + 8 chunks); cached meshes 974,064 B; snapshot JSON
  65,464 B. Excludes map/allocator overhead, instance records, GPU buffers and renderer
  state.
- meshes: mushroom 1,224 / 356 / 164 triangles at LOD 1/2/4; tile 7,202 / 2,044 / 606
  triangles. Every mesh reports its source revision (938 / 25,175).
- `ground_contact` in the manifest: all six mushrooms report `gap_m` 0.0 on `moss_turf`
  columns (previously 1.25, -0.25, 0.25, 0.25, 0.5, -0.25 m).
- host-only timing (`timing.json`): scene build 18.40 ms; six derived meshes measured
  individually. HOST ONLY, not phone performance.
- `sha256-a2.txt` is a plain `sha256sum` sidecar over the manifest, mesh buffers and
  snapshots (15 files); the lead's hashing script can replace it without a crate
  dependency.

## Limits and what is open

- Water is stored as an opaque material, so the core mesher culls faces between water and
  terrain. That is correct for a solid surface but gives no separate transparent water
  surface; a render adapter needs a water pass or a separate volume later.
- Quarter-turn yaw only: no pitch, roll or non-uniform scale. Fractional-metre translation
  is supported; sub-cell *rotation* alignment is not.
- One fixture flora prototype (parasol mushroom) and one 16 m tile. Catalogue breadth,
  clustering and habitat rules are follow-up work.
- Scene point queries scan every instance linearly (collision now deliberately visits all
  overlaps); there is no spatial index yet, so query cost grows with instance count.
- Budgets are per-volume/per-scene counters, not a general memory framework, and the mesh
  bound is a pessimistic upper bound rather than a measured estimate.
- No streaming, no async meshing and no collision-shape export in this crate.
- All numbers here are desktop host measurements. No phone, GPU or visual check was run by
  this worker.
- Lead correction: the final adapter counts retained mesh vector capacities and reserves
  each appended chunk exactly, including both vertex and index buffers. A conservative
  source-cell upper bound is checked before building a new mesh or coarsening vote map;
  this may reject a large, easily meshed dense volume. Split/prototype-budget refinement
  requires native evidence.
- Temporary core chunk-mesher data, coarse source copies, map entries and allocator
  metadata are additional to the output/cache byte budgets. With the current preflight,
  coarsening processes at most 33,288 source cells; these are bounded prototype
  operations, not a general-purpose full-world LOD system.
- Lead correction: scene queries validate finite scene-space coordinates even without
  instances. A valid point beyond one small prototype's local coordinate range is a miss
  for that instance, so a distant earlier instance cannot mask a nearby one. Volume-local
  query APIs retain their explicit out-of-range error. Placement spacing checks both
  horizontal axes during search. Unchanged scene edits preserve cache entries; edits
  creating new chunks obey the aggregate source cap before mutation.
