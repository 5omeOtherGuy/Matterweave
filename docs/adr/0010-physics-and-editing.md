# ADR-0010: Physics, destruction and editable collision

- Date: 2026-09-07
- Status: Proposed
- Basis: Proposed foundation for the owner's complex-physics interest.
- Requirements: R07, R12

## Context

Rigid bodies, constraints, characters and editable collision form a useful first physics capability. Fine visual voxels cannot all become independent simulated bodies without considering cost. Generic physics libraries do not automatically solve voxel fracture, connectivity or collision regeneration.

## Decision

Evaluate Jolt as a native rigid-body/constraint foundation. Keep Matterweave's body/query interface narrow and separate from world/render LOD. Build collision proxies appropriate for static terrain and movable objects, with bounded updates and explicit edit-to-collision publication semantics.

Use a fixed simulation step, bounded catch-up and render interpolation. Start destruction with connected rigid fragments, tracking mass/inertia and collision changes. Sleep or simplify noncritical simulation without changing gameplay-critical outcomes based on camera position.

## Alternatives

Bullet or another mature library remains eligible if integration evidence favors it. A custom collision/fracture layer may be needed around the library. A complete custom solver, GPU physics or continuum simulation is a later decision requiring specific evidence.

## Consequences

Fragment counts, collider preparation, collision latency and worker scheduling need budgets. Rendering and physics should agree at defined publication points. Deterministic generation does not establish cross-device deterministic physics. Fluid/soft-body features are not implied by adopting a library that may expose them.

## Validation

M3 demonstrates constraints, dynamic voxel bodies and a breakable structure; tests edits, stale collider jobs, LOD independence, mass properties, persistence and stability under load. Measure both steady simulation and collider/fracture preparation costs.

## References

[Jolt source and platform support](https://github.com/jrouwe/JoltPhysics), [world data](0005-voxel-world-data.md), [architecture](../ARCHITECTURE.md).
