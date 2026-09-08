# Detail-scene static collision — execution log

Worker session, worktree `worktrees/completion-collision`, starting commit `c6e2317`.
Host build only. No device measurement was run, so no mobile claim is made here.

## Outcome

`matterweave_physics::Physics::replace_detail_scene(&DetailScene) -> Result<DetailCollisionStats, String>`
publishes all static collision derived from a detail scene at load/edit time. An
empty scene clears it. Nothing regenerates per frame.

Files:

- `crates/matterweave-physics/src/detail_collision.rs` (new module and API)
- `crates/matterweave-physics/tests/detail_collision.rs` (new behavior tests)
- `crates/matterweave-physics/src/lib.rs` (module, re-exports, two struct fields, `new` init)
- `crates/matterweave-physics/Cargo.toml` + `Cargo.lock` (path dependency on `matterweave-detail`)

## Actions taken

1. Read `AGENTS.md`, `docs/SHOWCASE.md`, `crates/matterweave-physics/src/lib.rs`
   (`solid_boxes`, `sync_world`, `fixed_step`, character controller use) and
   `crates/matterweave-detail/src/{lib.rs,scene.rs}`.
2. Inspected the pinned backend first: `rapier3d 0.32.0` / `parry3d 0.26.1` in the
   cargo registry, specifically `SharedShape::voxels`, `shape/voxels/*` storage and
   the `DefaultQueryDispatcher` branches for `ShapeType::Voxels`.
3. Implemented the voxel-shape backend, ran the behavior tests, found the character
   defect below, re-measured, and switched to greedy merged cuboids.
4. Ran the scoped test suite and clippy; recorded the `dense_tile` cost.

## Backend comparison (measured on this pin)

- `parry::shape::Voxels` storage is genuinely sparse: 8x8x8 chunks, one state byte
  per voxel, plus a chunk BVH; far-apart occupied regions do not allocate the space
  between them (`src/shape/voxels/voxels.rs::new`, `voxels_chunk.rs`).
- Rejected anyway. `contact_manifolds_voxels_shape` intersects the *unloosened*
  voxel AABB with the other shape's AABB before generating contacts, so it emits
  nothing across the character controller's prediction gap. Probe with this pin,
  capsule (0.55 half-height, 0.30 radius) 5 mm above a 0.25 m voxel slab:
  - capsule vs `SharedShape::voxels`: `contact_manifolds` → `Ok(())`, **0 manifolds**
  - capsule vs `SharedShape::compound([cuboid])`, same pose: **1 manifold**,
    `local_n1 = (-0, -1, -0)`, contact dists `[0.005, 1.105, 0.005]`
  Consequence observed end to end: the character rested on the voxel slab at
  eye `1.7671` (expected `~1.75`) but reported `grounded == false` forever, so it
  could never jump. Direct `intersection_test` and `cast_shapes` against `Voxels`
  did work, so the defect is specific to contact manifolds / grounding.
- Chosen: greedy merged cuboids in one `SharedShape::compound` per prototype. The
  merge order (x, then z, then y) is the same idea as the existing terrain
  `solid_boxes`, but that function is not reusable: it is hard-wired to a dense
  16^3 `matterweave_core::World` chunk, 1 m cells and "material != 0 is solid",
  while detail prototypes are sparse, own their cell scale (6.25 cm / 25 cm) and
  must filter by `material_policy`. The new merge runs over a `BTreeSet` of
  collision cells, so cost follows occupied cells, not the bounding box.

## Measured cost of the existing authored fixture

`dense_tile(FLORA_CANONICAL_SEED)`, host build (`cargo test`, `dev` profile with
this repo's optimized test settings), single call, printed by
`dense_tile_collision_cost_stays_bounded`:

```
instances=85 colliders=43 shapes=4 source_collision_cells=27532
merged_boxes=1217 expanded_collision_cells=66296 build=18.053224ms
```

So: 4 shared shapes for 43 static colliders; 1,217 cuboids describe 27,532 source
collision cells (22.6x reduction); 42 of the 85 instances are decorative/liquid and
get no collider at all. Host figure only; no device timing was taken.

## Policy and limits

- Finest authoritative source cells only; derived meshes and LOD never participate.
- `MaterialPolicy::Collision` cells only. Water and decorative flora are never walls.
- Every overlapping instance is evaluated, so a decorative or liquid volume in front
  of a solid one cannot mask it.
- One shape per prototype, shared by all its static transformed instances. Shapes are
  rebuilt on every call, so a scene restored to an earlier revision can never reuse a
  stale shape.
- Conservative limits are checked before allocation: `MAX_DETAIL_COLLIDERS` (16,384),
  `MAX_DETAIL_PROTOTYPE_COLLISION_CELLS` (1,048,576, pre-checked against the O(1)
  occupied-cell count), `MAX_DETAIL_SOURCE_COLLISION_CELLS` (4,194,304) and
  `MAX_DETAIL_BOXES` (262,144).
- Transactional: any rejection returns `Err` before a single collider is touched, so
  an invalid or over-budget update fails loudly instead of dropping walls.
- After a successful replacement, voxel bodies overlapping the changed region (union
  AABB of removed and added colliders, loosened 0.5 m) are woken; the count is reported.

## Verification (executed)

- `cargo test -p matterweave-physics --locked` (offline, `CARGO_BUILD_JOBS=2`,
  private target dir): 42 passed, 0 failed, across 5 suites.
- `cargo clippy -p matterweave-physics --all-targets --locked -- -D warnings`: clean.
- New tests cover: falling body resting on detail collision; character floor support,
  wall blocking and cap-blocked jump; all four yaws with negative-coordinate
  placements cross-checked against `DetailScene::is_collidable_world_metres`;
  liquid/decorative instances producing no colliders; a decorative instance failing to
  mask an overlapping solid; shared-shape reuse across 16 instances; prototype edit,
  revision restore and instance removal; body wake-up; LOD independence; over-budget
  rejection preserving previous colliders, stats and character contact; repeated
  clearing; legacy terrain regression plus no per-frame regeneration.

## Issues, friction and decisions

- The voxel-shape path cost one implement/measure/revert cycle. Recorded above rather
  than hidden, because the storage argument for `Voxels` is still valid and only the
  contact-manifold grounding behavior blocks it on this pin.
- Two early test failures were test bugs, not engine bugs: teleport probes placed the
  capsule bottom inside the slab or inside legacy terrain (the capsule extends 0.85 m
  below the centre). Corrected in the tests.
- No device, APK or renderer work was attempted here.

## Not done / next

- No integration into the explorer app or Android path; `replace_detail_scene` has no
  caller outside tests yet.
- No device measurement; the 18 ms figure is host-only.
- Detail colliders are a single flat set replaced as a whole. Per-region incremental
  replacement is possible future work if edit latency on a full showcase scene proves
  too high.
