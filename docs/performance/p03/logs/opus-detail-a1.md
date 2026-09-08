# Engineering log — P03 detail foundation (worker A1)

Base commit `94dc2ca`, worktree `worktrees/performance-detail`,
`CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/desktop-01/target`.

## Actions taken

1. Read `AGENTS.md`, `docs/SHOWCASE.md`, `crates/matterweave-core/src/{lib.rs,mesh.rs}`
   and the core/persistence public surface to decide reuse before writing code.
2. Added `crates/matterweave-detail` (lib, tests, example) and one workspace
   member entry; `Cargo.lock` gained the new local package only.
3. Implemented `Scale`, `Yaw`/`Transform`, `Bounds`, palette + policy,
   `DetailVolume` (storage/mesh/LOD over `matterweave_core::World`),
   `DetailScene` (prototypes, instances, revision-keyed mesh cache, counts),
   run-length `VolumeSnapshot`, and integer-only fixtures (16 m 0.25 m tile with
   creek/water and a 0.0625 m parasol mushroom).
4. Wrote 16 integration tests and the `detail_gallery` example, then ran fmt,
   tests and clippy.

## Commands and results

- `cargo build -p matterweave-detail --locked` → failed: lock file needed the new
  member. Re-ran with `--offline` to update the lock without network, then
  `--locked` worked for the final run.
- `cargo test -p matterweave-detail --locked` → `16 passed (3 suites, 0.05s)`.
- `cargo clippy -p matterweave-detail --all-targets --locked -- -D warnings` →
  clean after fixing two lint findings in tests (`needless_range_loop`,
  `assertions_on_constants`).
- `cargo fmt -p matterweave-detail` → applied to owned files only.
- `cargo run -p matterweave-detail --release --example detail_gallery` →
  manifest written to
  `/mnt/bench/matterweave-dev/performance/desktop-01/detail-gallery/manifest.json`.
  Measured (HOST ONLY): unique stored cells 26,113; expanded occupied 30,803
  (collision 30,204, liquid 599); source payload 163,840 B; derived meshes
  974,064 B; snapshots 65,464 B; tile generation 4.26 ms; mushroom 0.14 ms; six
  derived meshes 30.04 ms.

## Issues and friction

- Core's greedy mesher colours vertices from its own 8-entry palette, and our
  material IDs (10–23) all fall through to its default colour. Recovering the
  material from a raw vertex position is wrong for greedy quads because corners
  lie on cell boundaries.
- `Mesh` does not implement `Clone`, so the cache entry struct could not derive it.
- `--locked` blocks the first build of a brand new workspace member.

## Decisions and rationale

- Reuse `World` for storage and `mesh_chunk` for meshing rather than writing a
  mesher: seam face removal, determinism and revision invalidation already exist
  and are tested; a parallel implementation would be redundant risk.
- Restrict transforms to quarter-turn yaw plus translation. That is enough for
  flora placement, keeps normals axis aligned, makes bounds exact and makes
  queries an exact inverse map — no arbitrary affine machinery.
- Water stays authoritative source data with a `Liquid` policy instead of being
  removed from collision by a special case at the call site, so the rule is one
  table lookup shared by queries and documentation.
- Coarsening produces a separate derived volume rather than mutating in place, so
  "source preserved" is structurally true and directly testable.
- Snapshots use X-runs with an explicit bound instead of a per-cell blob, per the
  brief's ban on unbounded textual dumps.

## Solutions applied

- Palette fix: one lookup per greedy quad, sampling half a cell inward from the
  quad centroid along `-normal`, which is always inside the owning solid cell.
- Cache entry stores `Mesh` without `Clone`; only the map owns it and callers get
  a reference.
- Ran `--offline` once to add the lock entry, then verified with `--locked`.
- Gill blades use a fixed table of 16 integer directions instead of `atan2`, so
  generation is bit-identical across targets.

## Insights

- Reported memory must name its exclusions: source payload (163,840 B) is small
  next to derived meshes (974,064 B) at this scale, so any "voxel memory" claim
  that ignores derived geometry is misleading.
- Unique vs expanded counts differ by only ~18% here because one large tile
  dominates; instance reuse pays off only as flora counts grow, which is the
  argument for the catalogue work, not for duplicating prototype bytes.
- Six mushroom placements cost exactly one mesh build (`derived_mesh_builds` is
  6 = 2 prototypes x 3 LODs), which is the concrete evidence for the
  no-per-instance-meshing requirement.

## Not run by this worker

Full workspace build/tests, Android/native/visual checks and any phone
measurement. No commits, pushes or network changes.
