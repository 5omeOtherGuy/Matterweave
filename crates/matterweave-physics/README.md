# Voxel physics adapter

This Rust crate supplies terrain collision, a walking capsule, rigid voxel volumes,
constraint-based grabbing, throwing and bounded voxel fracture. Its public API uses
arrays and Matterweave core types; no Rapier, window or Vulkan handles escape.

Rapier **0.32.0**, Apache-2.0, is pinned from
[crates.io](https://crates.io/crates/rapier3d/0.32.0) and maintained by
[Dimforge](https://github.com/dimforge/rapier). The integration inspected its
`PhysicsPipeline`, `KinematicCharacterController`, compound collider, spring joint
and shape-query source, alongside the official
[character controller documentation](https://rapier.rs/docs/user_guides/rust/character_controller/).
It supplies the needed Rust rigid-body solver, CCD, mass/inertia calculation,
shape casting and constraints without a native C++ binding. This is a provisional
workload integration, not a performance win over Jolt or a claim that every future
physics requirement is met. No credible requirement gap justified a Jolt binding
experiment for this slice. The lockfile records all transitive revisions/checksums.

## Contracts

- Coordinates are meters, Y up. Solid terrain cells are one meter. The standing
  capsule is 1.7 m tall; its eye is 1.5 m above its feet.
- `sync_world` publishes exact solid-voxel collision from loaded 16³ chunks. It
  merges occupied cells into solid boxes and builds one compound per chunk.
  Collision uses authoritative cells, independently of visual meshes and LOD.
  Changed revisions replace colliders synchronously; evicted chunks are removed.
  The next fixed step updates broad-phase queries before moving the character.
  Teleport uses direct intersection checks, including newly published colliders.
- Simulation runs at 60 Hz with at most six catch-up steps per call. Excess stalled
  time is discarded; nonfinite/negative dt is ignored. Horizontal speed is bounded
  to 12 m/s, falling character speed to 35 m/s, and jump edges survive sub-step
  frames. Dynamic rendering interpolates the two latest fixed poses.
- Walking uses Rapier move-and-slide, ground snapping, gravity and an 8 m/s jump.
  Character impulses push dynamic objects. Autostep is disabled: full-meter steps
  require jumping. X/Z position stays within the finite world's ±256 m boundary.
  The application owns respawning after the character falls below the world.
- Dynamic bodies are solid half-meter voxel volumes (each dimension 1 or 2),
  with density-derived mass and inertia and CCD enabled. A break converts the
  volume into its original voxels, carrying rotation and point velocities forward
  with a small outward fracture impulse. It is a bounded voxel split, not a
  connectivity/stress fracture solver. Up to 64 bodies are retained. A fracture
  exceeding the cap is refused before mutation.
- Grabbing uses a spring joint to a kinematic anchor three meters along the view;
  throwing adds a mass-scaled impulse. Terrain ray hits occlude interaction with
  bodies. Held joints are transient and released on restore or teleport; a grab
  releases when stretched more than 12 m from the character. Simulation caps
  linear speed at 128 m/s and angular speed at 64 rad/s, keeping strong impacts
  inside the validated persistence bounds.
- Bodies farther than 40 m in X/Z are disabled with their poses/velocities intact,
  before streamed terrain can be evicted (core retains at least 48 m). Returning
  re-enables them. Bodies falling below the world are retained inactive at Y=-32
  with zero velocity, preventing unbounded void acceleration. This is a
  player-distance activity policy, not visual LOD.
- `PhysicsSnapshot` validates version, count, finite bounds, quaternion norm,
  voxel dimensions and materials before changing live state. The caller owns
  atomic storage alongside authoritative world data. A saved character pose is
  restored exactly; interactive teleport instead rejects intersecting geometry.
- `step_objects` is the application's fly-mode path. It temporarily disables the
  character collider and advances dynamic simulation without character gravity.
  The caller sets `set_flying_eye` first, which permits passage through solids
  without resetting accumulated time. Use collision-checked `teleport` when
  switching back to walking.

## Verification and limits

Run `cargo test -p matterweave-physics` for behavioral floor/wall/jump tests,
negative chunk boundaries and collision edits, invalid input/time bounds,
terrain-occluded grabbing and throwing, bounded fracture, atomic snapshot
validation, and dynamic-body preservation across terrain eviction.

The adapter does not claim deterministic cross-device physics, stress-driven
fracture, articulated voxel creatures, fluids, soft bodies, or mobile thermal and
large-body-count efficiency. Host behavioral tests establish correctness of these
bounded scenarios; APK operation and sustained device measurements belong in the
repository's release evidence.
