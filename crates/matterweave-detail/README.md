# matterweave-detail

Fine-scale authoritative detail volumes, prototypes and instanced scenes with derived
level-of-detail copies, on top of the sparse `matterweave-core::World`.

## What it provides

The crate adds a local fine cell scale (metres per cell) without rescaling the global world,
physics, save format or material IDs. A `DetailVolume` owns an id, a `Scale` and a sparse
voxel world; a `DetailScene` owns prototypes and transformed instances and derives meshes and
LODs per prototype, never per instance.

- **Authoritative source.** `DetailVolume` is the only authoritative geometry. Local cell
  coordinates are signed `[i32; 3]`; cell `c` occupies `[c * scale, (c + 1) * scale]`. A
  `Transform` is a quarter-turn yaw about local +Y followed by a finite, bounded translation
  in world metres, so fractional placement is supported and only rotation is restricted.
- **Derived LOD.** `Lod::Source`, `Half` and `Quarter` are derived copies; `DetailVolume::coarsen`
  never mutates the source, and a derived `Mesh` always carries the authoritative source
  revision. A coarse cell is occupied when any source cell inside it is occupied, and its
  material is the majority vote with ties to the lowest ID.
- **Selection.** `DetailScene::select_lods` chooses a level per instance from a `Camera` and
  `LodConfig` using a screen-space error *estimate*, hysteresis, a thin-feature dilation bias
  and a local interior-loss guard. `DetailScene::prepare_batches` synchronously realizes the
  needed meshes within budget and groups them into `MeshBatch`es. The authoritative source is
  never touched by selection or preparation.
- **Collision policy.** `material_policy` classifies each material as `Collision`, `Liquid` or
  `Decorative`. Only collision cells become walls; liquid and decorative cells never do, so a
  lush decorative scene attaches no colliders. Scene queries consider every overlapping
  instance, so a decorative or liquid volume in front of a solid one cannot mask it.
- **Snapshots.** `VolumeSnapshot` is a deterministic X-run encoding in stable iteration order;
  `DetailVolume::snapshot`, `from_snapshot` and `from_json_bytes` are the round trip.
- **Content.** Deterministic generators for a terrain tile, a parasol mushroom, a gallery
  scene, the additive flora catalogue, the wetland species and a 128 m showcase map.

## Public surface

| Group | Items |
| --- | --- |
| Volume and transforms | `DetailVolume`, `Scale`, `Transform`, `Yaw`, `Bounds`, `Lod`, `DetailError`, `Result` |
| Palette | `material` module (`DETAIL_SOIL` … `FLORA_FUNGUS_LUMEN`), `material_name`, `material_policy`, `material_color`, `MaterialPolicy` |
| Scene | `DetailScene`, `InstanceDraw`, `SceneCounts`, `SceneVersion` |
| Selection and preparation | `Camera`, `Projection`, `LodConfig`, `ErrorMetrics`, `InstanceLod`, `MeshBatch`, `PreparedFrame`, `LocalTopologyCost` |
| Snapshot | `VolumeSnapshot`, `SNAPSHOT_VERSION`, `MAX_SNAPSHOT_RUNS`, `MAX_SNAPSHOT_JSON_BYTES` |
| Fixtures | `terrain_detail_tile`, `parasol_mushroom`, `column_top`, `gallery_scene`, `FIXTURE_GENERATOR_VERSION` |
| Flora | `flora_prototype`, `funnel_mushroom`, `clustered_mushroom`, `fan_frond`, `reed_cluster`, `rosette_groundcover`, `dense_tile`, `instance_support`, `flora_class`, `assert_policy`, `FLORA_SPECIES` and its generator constants |
| Wetland and bracket | `wetland_prototype`, `twisted_shrub`, `horsetail`, `marsh_lily`, `bracket_fungus`, `WETLAND_FLORA_SPECIES` |
| Showcase | `build_showcase`, `showcase_prototype`, `showcase_class`, `composition_hash`, `Terrain`, `SurfacePoint`, `Showcase`, `Landmark`, `ClassCounts`, `ContentManifest`, `terrain_height_m`, `carved`, `species_source_radius_m` |

Key bounds, all checked before allocating:

| Constant | Value | Meaning |
| --- | --- | --- |
| `MIN_SCALE_M` / `MAX_SCALE_M` | 0.001 / 1.0 | Accepted cell size in metres. |
| `SCALE_FINE_M` / `SCALE_TILE_M` | 0.0625 / 0.25 | Flora and terrain/tile scales. |
| `MAX_CELL_COORD` | 65536 | Inclusive local cell bound per axis. |
| `MAX_VOLUME_CHUNKS` | 1024 | Resident chunks per volume (4 MiB of source payload). |
| `MAX_MESH_BYTES` | 32 MiB | Vertex/index output bound for one derived mesh. |
| `MAX_PROTOTYPES` / `MAX_INSTANCES` | 4096 / 200000 | Per scene. |
| `MAX_SCENE_TRANSLATION_M` | 65536.0 | Plausible world extent for a transform, in metres. |
| `MAX_SCENE_SOURCE_BYTES` / `MAX_SCENE_CACHE_BYTES` | 32 MiB / 64 MiB | Aggregate authoritative payload and derived mesh cache. |
| `MAX_SNAPSHOT_RUNS` / `MAX_SNAPSHOT_JSON_BYTES` | 2 000 000 / 16 MiB | Snapshot run count and pre-parse byte cap. |
| `LOCAL_TOPOLOGY_CELL_BUDGET` | 8192 | Coarse cells analyzed per prototype and factor for the interior-loss guard. |

## Invariants and guarantees

- The global world, physics, save format and material IDs are never rescaled. Detail
  materials are a separate local palette.
- Derived representations never mutate or replace authoritative source data. Edits change the
  source revision, and `prototype_mesh` caches keyed by `(prototype, LOD, revision)`.
- Complete-validate-then-mutate: a rejected operation (budget, identity, transform, snapshot)
  leaves prior data, caches and revision accounting unchanged. Snapshot loading validates
  every run's range, length, material, budget and overlap before writing any voxel.
- Deterministic: stable `BTreeMap` iteration order, majority-vote ties to the lowest material
  ID, and a snapshot encoding that produces identical bytes for identical content.
- `MaterialPolicy` is enforced: decorative and liquid cells are never physical walls, and a
  collision query checks every overlapping instance.
- No `unsafe` code. Dependencies: `matterweave-core`, `bytemuck`, `serde`, `serde_json`.

## Limits and what it does not do

- The budgets are conservative prototype budgets chosen to hold the current fixtures on a
  desktop host. They are not a claim about phone feasibility, process memory or mobile
  residency.
- Error values are *estimates*, not guarantees: `error_estimate_m` is the coarse cell size, and
  any-occupied coarsening can fill a small deep opening. The `local_loss_fraction` guard holds
  a prototype with a partially filled interior coarse cell at the finer level, but it is a
  bounded local topology test (per prototype and factor, cached by source revision), not a
  proof of global topology preservation.
- Transforms and queries are `f32`. Quarter-turn rotation and translation add no rounding
  beyond ordinary `f32` addition, but a query point exactly on a cell boundary resolves by that
  addition; no exactness is claimed for arbitrary floating-point inputs. Out-of-range points
  and cells are rejected, never silently clamped.
- LOD selection and mesh preparation are synchronous and budgeted in-process, and
  `MAX_SCENE_CACHE_BYTES` is an error ceiling, not an eviction policy: exceeding it fails the
  build rather than silently evicting. A coarse build can fall back to `Source` when the build
  cap or cache budget is reached; the fallback is flagged in `InstanceLod::fallback`.

### Not implemented

[ADR-0007](../../docs/adr/0007-virtualized-detail.md) also describes capabilities this crate
has not built:

- Asynchronous detail preparation or cancellation.
- A transition or blending strategy between LOD levels.
- Temporal-stability validation and equivalent-quality mobile LOD comparison evidence.

## How it is tested

From the repository root:

```sh
cargo test -p matterweave-detail
```

The crate has 12 integration suites under `tests/` plus inline unit tests covering:

- volume editing, negative coordinates, transform round trips and boundary queries
  (`detail.rs`);
- greedy mesh seam removal and winding, revision/cache invalidation, snapshot round trips and
  malformed-snapshot rejection (`detail.rs`);
- generator anatomy, determinism, budgets and material policy (`flora.rs`,
  `wetland_flora.rs`, `bracket.rs`);
- the local interior-loss guard holding tunnels, pinholes and thin sheets at `Source`
  (`local_loss.rs`);
- selection semantics: view-forward depth, orthographic zoom, hysteresis, quality cap and
  numeric rejection (`lod_selection.rs`, `lod_correctness.rs`, `lod_numeric.rs`);
- the full-map showcase content and walkable-route checks (`showcase.rs`);
- instance-local edits and source-version identity (`instance_edits.rs`, `source_snapshot.rs`,
  `source_version.rs`).

Host examples `detail_gallery`, `flora_gallery`, `wetland_flora_gallery`, `lod_sweep` and
`showcase_manifest` exercise the content and selection paths. None of this is a device or
mobile-performance measurement; native capture review remains owed (see
[ADR-0007](../../docs/adr/0007-virtualized-detail.md)).
