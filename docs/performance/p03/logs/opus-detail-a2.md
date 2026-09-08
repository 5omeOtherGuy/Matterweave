# Engineering log — P03 detail repair (worker A2)

Base commit `94dc2ca`, worktree `worktrees/performance-detail`,
`CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/desktop-01/target`.
Scope: repair the nine verified defects in the A1 foundation. No new
dependencies, no flora/map expansion, no commits, no device work.

## Actions taken

1. Cell/point validation: `check_cell` now uses signed range membership
   (`(-MAX_CELL_COORD..=MAX_CELL_COORD).contains`), so `i32::MIN` is rejected
   instead of panicking on `abs`. Added `DetailError::InvalidPoint`;
   `cell_at_local_metres`, `sample_local_metres`, `sample_world_metres`,
   `mesh_world` and `bounds_world` return `Result` and reject NaN/infinite/
   out-of-range points and unvalidated `Transform`s (`Transform::is_valid` /
   `validate`).
2. Snapshot loading is two-phase: all runs validated (length, material, cell
   range, overflow), totals aggregated against `occupied_cells`,
   `MAX_VOLUME_CELLS` and a distinct-chunk count, and overlaps rejected on a
   sorted copy *before* any payload write. Added bounded `from_json_bytes`
   (`MAX_SNAPSHOT_JSON_BYTES`) and documented that other decode paths are
   caller-bounded.
3. `coarsen` returns `Result`, propagates `set` failures, and returns
   `InvalidScale` for unsupported derived scales instead of clamping.
   `prototype_mesh` overwrites the coarse mesh revision with the authoritative
   prototype revision.
4. Removed `prototype_mut`; added `DetailScene::edit_prototype(id, cell,
   material)` and `invalidate(id)`, which preserve identity and unconditionally
   drop cached meshes.
5. `is_collidable_world_metres` now scans every overlapping instance; added
   `sample_all_world_metres`; generic `sample_world_metres` keeps its documented
   first-hit policy.
6. Budgets checked before allocation: `MAX_VOLUME_CHUNKS` (1024 = 4 MiB/volume),
   `MAX_SCENE_SOURCE_BYTES` (32 MiB), `MAX_SCENE_CACHE_BYTES` (64 MiB),
   `MAX_MESH_BYTES` (32 MiB, from a 24-vertices-per-cell upper bound), snapshot
   run/byte caps. Coarsen votes switched from `[u32; 256]` per coarse cell to a
   compact association list.
7. `occupied_cells`/`chunk_count` maintained incrementally in `set`
   (O(log n) chunk lookups); no `world.stats()` scan per inserted cell.
8. Scene composition moved into `gallery_scene(seed)`; placements derived from
   source queries (`column_top`, footprint-wide dry `moss_turf` footing) instead
   of hand-tuned offsets.
9. Example split into deterministic `manifest.json` (content, budgets, ground
   contact) plus `timing.json` (host only), with explicit little-endian vertex
   and index export and a `sha256sum` sidecar.

## Issues and friction

- The first placement search was O(volume) per candidate column, because
  `column_top` recomputed `cell_bounds`; the test suite took 30 s.
- `Mesh` has no `Clone`, so the cache entry could not derive `Clone` once byte
  accounting was added.
- Making fixtures fallible rippled through every test and the example; the API
  surface changed more than a pure bug fix would, which is why the API-change
  list is called out for the integration lead.

## Decisions and rationale

- `Result`-returning queries rather than `Option`: "invalid input" and "air" are
  different answers, and the earlier `Option`/`u8` shape is exactly what let NaN
  read as an origin hit.
- Refuse rather than clamp for derived scales, and refuse rather than evict for
  cache budgets: silent adjustment is what produced the shrunken LOD and the
  masked collision defects in the first place.
- Collision considers all overlaps while generic sampling stays first-hit: the
  physical question is "is anything solid here", the inspection question is "what
  do I see first"; conflating them was defect 5.
- Budget values are per-volume/per-scene counters sized for present fixtures, not
  a memory framework: the tile needs 32 chunks against a 1024-chunk cap.
- Placement derives from authoritative queries with a footprint-wide equal-height
  check, so a mushroom cannot straddle a step or a bank.

## Solutions applied

- Added `column_top_in` with a precomputed vertical range: suite back to 0.15 s
  for the placement tests, 2.0 s total.
- Cache entry stores `Mesh` plus its byte size without `Clone`.
- Tests hit budgets with sparse one-cell-per-chunk patterns (1024 and 8×1024
  chunks), never gigabytes of dense data.

## Insights

- Every defect except the placement gap was a *silent-adjustment* bug: clamp,
  first-hit, `abs`, declared-count trust. Returning errors made four of them
  testable in a single pass.
- Ground contact is now evidence in the artifact (`gap_m` 0.0 for all six
  instances, all on `moss_turf`), not an assertion in prose.
- Separating timing from content made the manifest byte-comparable; without that
  split no manifest hash could ever reproduce.

## Commands and results

- `cargo test -p matterweave-detail --locked` → `31 passed (3 suites, 2.00s)`.
- `cargo clippy -p matterweave-detail --all-targets --locked -- -D warnings` → clean.
- `cargo fmt -p matterweave-detail -- --check` → clean.
- `cargo run -p matterweave-detail --release --example detail_gallery` →
  `desktop-01/detail-gallery-a2/` (manifest.json, timing.json, 12 `*.le.bin`,
  2 snapshots, sha256-a2.txt). Unique stored cells 26,113; expanded 30,803
  (collision 30,204, liquid 599); source payload 163,840 B; cached meshes
  974,064 B; scene build 18.40 ms HOST ONLY.
- `python3 tools/check_docs.py` → PASS.

## Not run by this worker

Full workspace build/tests, Android/native/visual checks, any phone measurement,
and `gallery-acceptance.md` (left unchanged). No commits or pushes.
