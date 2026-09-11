# matterweave-physics

Fixed-step voxel physics adapter: terrain collision, a walking capsule, rigid voxel bodies,
constraint-based grabbing and throwing, bounded voxel fracture, and static detail collision
derived from authoritative detail cells. The public API uses arrays and Matterweave core/detail
types; no Rapier, window or Vulkan handle escapes.

## What it provides

| Area | What it is |
| --- | --- |
| Terrain collision | `sync_world` publishes exact solid-voxel collision from loaded 16³ chunks as one compound of merged boxes per chunk. |
| Character | A 1.7 m capsule with an eye 1.5 m above its feet, moved by Rapier move-and-slide at a fixed 60 Hz step. |
| Dynamic bodies | Solid half-metre voxel volumes with density-derived mass/inertia and CCD; bounded fracture into the original voxels. |
| Interaction | A spring-joint grab, a mass-scaled throw, terrain-occluded ray interaction, and fly-mode `step_objects`. |
| Detail collision | Static collision built from authoritative `DetailScene` cells, synchronously or on one background worker, with an edit-to-publication cadence controller. |
| Dynamic cache | `DynamicMeshCache` retains interpolated dynamic geometry while its complete render inputs are unchanged. |

### Simulation and body contracts

- Coordinates are metres, Y up. Solid terrain cells are one metre. The standing capsule is
  1.7 m tall; its eye is 1.5 m above its feet.
- `sync_world` merges occupied cells into solid boxes and builds one compound per chunk.
  Collision uses authoritative cells, independently of visual meshes and LOD. Changed
  revisions replace colliders synchronously; evicted chunks are removed. The next fixed step
  updates broad-phase queries before moving the character. Teleport uses direct intersection
  checks, including newly published colliders.
- Simulation runs at 60 Hz (`FIXED_DT`) with at most six catch-up steps per call. Excess
  stalled time is discarded; nonfinite/negative dt is ignored. Horizontal speed is bounded to
  12 m/s, falling character speed to 35 m/s, and jump edges survive sub-step frames. Dynamic
  rendering interpolates the two latest fixed poses.
- Walking uses Rapier move-and-slide, ground snapping, gravity and an 8 m/s jump. Character
  impulses push dynamic objects. Autostep is disabled: full-metre steps require jumping. X/Z
  position stays within the finite world's ±256 m boundary. The application owns respawning
  after the character falls below the world.
- Dynamic bodies are solid half-metre voxel volumes (each axis 1..=6 voxels, at most 32 voxels
  per body; `MAX_VOXEL_DIM`, `MAX_VOXELS_PER_BODY`), with density-derived mass and inertia and
  CCD enabled. A break converts the volume into its original voxels, carrying rotation and
  point velocities forward with a small outward fracture impulse. It is a bounded voxel split,
  not a connectivity/stress fracture solver. Up to 64 bodies (`MAX_BODIES`) are retained. A
  fracture exceeding the cap is refused before mutation. Older `[1, 2]`-only snapshots still
  validate, so no snapshot version bump was needed for larger bodies.
- `spawn_playground` builds the v0.3 breakable arch in front of the home camera (near
  z 16..19 for the v0.3 scene) only when no bodies exist: two pillars of two 1 m wood (8)
  cubes, one bridging 3x1x1 m wood beam (`[6, 2, 2]` voxels) resting on probed equal-height
  terrain columns with 0.5 m overlap each side, and one loose 1 m accent (7) cube. Columns used
  by the `spawn_demo` stack are skipped. The 64 voxels total mean every body can fully
  fracture within the 64-body budget. `spawn_demo` is unchanged.
- Grabbing uses a spring joint to a kinematic anchor three metres along the view; throwing
  adds a mass-scaled impulse. Terrain ray hits occlude interaction with bodies. Held joints
  are transient and released on restore or teleport; a grab releases when stretched more than
  12 m from the character. Simulation caps linear speed at 128 m/s and angular speed at
  64 rad/s, keeping strong impacts inside the validated persistence bounds.
- Bodies farther than 40 m in X/Z are disabled with their poses/velocities intact, before
  streamed terrain can be evicted (core retains at least 48 m). Returning re-enables them.
  Bodies falling below the world are retained inactive at Y = −32 with zero velocity,
  preventing unbounded void acceleration. This is a player-distance activity policy, not
  visual LOD.
- Dynamic-body activation is additionally gated against columns from the published collision
  window, including resident air. A conservative rotation-independent radius and one
  bounded-speed fixed step supplement the 40 m activity radius. This preserves nearby objects
  when async terrain preparation lags behind the camera; the delayed-window regression
  verifies frozen objects resume after synchronous collision publication.
- `PhysicsSnapshot` validates version, count, finite bounds, quaternion norm, voxel dimensions
  and materials before changing live state. The caller owns atomic storage alongside
  authoritative world data. A saved character pose is restored exactly; interactive teleport
  instead rejects intersecting geometry.
- `step_objects` is the application's fly-mode path. It temporarily disables the character
  collider and advances dynamic simulation without character gravity. The caller sets
  `set_flying_eye` first, which permits passage through solids without resetting accumulated
  time. Use collision-checked `teleport` when switching back to walking.

### Detail collision contracts

- Only the authoritative finest source cells of a detail prototype are collision input; derived
  meshes and LOD never participate, so a coarse visual level can never delete a physical wall.
- Only `MaterialPolicy::Collision` cells become walls. Liquid and decorative materials are
  never walls at any level, so the 4,000-plant showcase does not attach a collider to every
  decorative plant. Every overlapping instance is considered independently, so a liquid or
  decorative instance in front of a solid one cannot mask it.
- One `SharedShape` is built per prototype and shared by every static instance; instances cost
  a transform and a collider handle, not new geometry. `PreparedDetailCollision::build` is
  isolated and transactional: a rejection returns `Err` before a single live collider is
  touched.
- `Physics::replace_detail_scene` is the synchronous convenience path. `publish_detail_scene`
  checks that the prepared source is still current and preserves live colliders on rejection.
  Nothing here runs per frame.
- `AsyncDetailCollision` prepares detail collision on one worker from an immutable scene
  snapshot, bounded to at most one pending snapshot, one running job and one buffered result,
  with latest-wins deduplication. `poll` returns a result only if it is current for the scene
  passed in and never blocks.
- `DetailCollisionCadence` is the per-frame integration seam: `on_edit` queues preparation from
  an immutable snapshot and `step` publishes at most one result per call. Publication of newly
  added solid material is gated while any dynamic body AABB overlaps an accumulated added
  region, so a new wall is never placed through a body; removals never gate. A blocked result
  is retained and retried, never dropped or forced through.
- Greedy merged cuboids in a `SharedShape::compound` are used. `SharedShape::voxels` was
  implemented and rejected against the pinned rapier3d 0.32.0 / parry3d 0.26.1: its manifold
  generation emits nothing across a small prediction gap, which left the kinematic character
  unable to jump. See `docs/performance/p03/collision/opus-execution.md`.

## Public surface

| Item | Notes |
| --- | --- |
| `Physics` | `new`, `sync_world`, `step`, `step_objects`, `teleport`, `set_flying_eye`, `intersects_character_cell`, `character_eye`, `grounded`, `held`, `body_count`, `body_activity`. |
| Dynamic interaction | `spawn_demo`, `spawn_playground`, `has_target`, `grab`, `update_grab`, `release`, `throw`, `break_body`, `dynamic_mesh`. |
| Persistence | `snapshot`, `restore`, `PhysicsSnapshot`, `BodySnapshot`. |
| Detail collision | `replace_detail_scene`, `publish_detail_scene`, `detail_added_blocked`, `detail_collision_stats`, `detail_collider_count`, `dynamic_body_aabbs`; `PreparedDetailCollision`, `DetailCollisionStats`, `DetailCollisionCadence`, `AsyncDetailCollision`, `AsyncDetailStats`. |
| Dynamic cache | `DynamicMeshCache` (`update`, `mesh`, `retained_bytes`). |
| Bounds | `FIXED_DT` (1/60), `MAX_BODIES` (64), `MAX_VOXEL_DIM` (6), `MAX_VOXELS_PER_BODY` (32), `MAX_DETAIL_COLLIDERS` (16 384), `MAX_DETAIL_SOURCE_COLLISION_CELLS` (16 << 20), `MAX_DETAIL_BOXES` (262 144), `MAX_DETAIL_SCRATCH_CHUNKS` (65 536), `MAX_PENDING_ADDED` (4 096). |

## Invariants and guarantees

- Collision always derives from authoritative cells, never from visual meshes or LOD, so a
  camera-only change cannot alter game rules or remove a wall.
- Preparation never touches a live `Physics` world: the worker builds from an immutable
  snapshot, and publication revalidates the source version. The simulation owner alone
  publishes.
- `sync_world` and publication are synchronous with respect to stepping; the cadence
  controller publishes at most one result per `step` and only when its gate is clear.
- `PhysicsSnapshot` and `PreparedDetailCollision` validate before mutating live state; rejected
  operations leave prior state and bodies unchanged.
- No `unsafe` code in the crate. `rapier3d 0.32.0` (Apache-2.0) is pinned from
  [crates.io](https://crates.io/crates/rapier3d/0.32.0); the lockfile records transitive
  revisions and checksums. See [DEPENDENCIES](../../docs/DEPENDENCIES.md) for the selection
  record.

## Limits and what it does not do

- The adapter does not claim deterministic cross-device physics, stress-driven fracture,
  articulated voxel creatures, fluids, soft bodies, or mobile thermal and large-body-count
  efficiency.
- The Rapier integration is a provisional workload integration, not a measured performance win
  over Jolt or a claim that every future physics requirement is met. No credible requirement
  gap justified a Jolt binding experiment for this slice.
- Fracture is a bounded voxel split, not a connectivity or stress solver.
- Detail collision is static per prototype with instance transforms; dynamic detail bodies are
  not derived from detail geometry. `SharedShape::voxels` is not used.
- The publication gate is conservative: it may defer a publication while a body overlaps the
  added region, and a structural (unknown) change latches a conservative mode that defers while
  any body overlaps any prepared collider. The visual/collision transition window has no fixed
  time bound.
- Host behavioral tests establish correctness of the bounded scenarios; APK operation and
  sustained device measurements belong in the repository's release evidence.

## How it is tested

From the repository root:

```sh
cargo test -p matterweave-physics
```

The suites cover behavioral floor/wall/jump tests, negative chunk boundaries and collision
edits, invalid input/time bounds, terrain-occluded grabbing and throwing, bounded fracture,
atomic snapshot validation, dynamic-body preservation across terrain eviction, playground arch
settling and support, full 64-piece fracture without rejected valid splits, and
voxel-dimension limit rejection. The detail-collision suites cover preparation limits, stale
publication rejection, the background worker, the edit-to-publication gate, and the dynamic
mesh cache. The delayed-window regression verifies frozen dynamic objects resume after
synchronous collision publication. `examples/async_detail_collision.rs` exercises the
background path.

The physics selection and evidence record is in
[ADR-0010](../../docs/adr/0010-physics-and-editing.md) and
[DEPENDENCIES](../../docs/DEPENDENCIES.md).
