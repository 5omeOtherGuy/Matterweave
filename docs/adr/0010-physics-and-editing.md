# ADR-0010: Physics, destruction and editable collision

- Date: 2026-09-07
- Status: Proposed
- Basis: Proposed foundation for the owner's complex-physics interest.
- Requirements: R07, R12, R20

## Context

Rigid bodies, constraints, characters and editable collision form a useful first physics capability. Fine visual voxels cannot all become independent simulated bodies without considering cost. Generic physics libraries do not automatically solve voxel fracture, connectivity or collision regeneration.

## Decision

Evaluate suitable Rust physics libraries first, with Rapier the initial candidate. Under accepted [ADR-0014](0014-rust-modularity-and-evidence-led-reuse.md), Jolt is eligible only if major workload-relevant advantages justify its Rust binding and integration costs. This replaces the earlier Jolt-first proposal; no solver has been selected. Keep Matterweave's body/query interface narrow and separate from world/render LOD. Build collision proxies appropriate for static terrain and movable objects, with bounded updates and explicit edit-to-collision publication semantics.

Use a fixed simulation step, bounded catch-up and render interpolation. Start destruction with connected rigid fragments, tracking mass/inertia and collision changes. Sleep or simplify noncritical simulation without changing gameplay-critical outcomes based on camera position.

## Alternatives

Existing Rust libraries, focused extensions and qualifying foreign dependencies are alternatives under the component-selection criteria. A custom collision/fracture layer may be needed around the library. Do not hand-roll a complete solver when an existing solution meets requirements; custom work requires necessity or a significant demonstrated advantage.

## Consequences

Fragment counts, collider preparation, collision latency and worker scheduling need budgets. Rendering and physics should agree at defined publication points. Deterministic generation does not establish cross-device deterministic physics. Fluid/soft-body features are not implied by adopting a library that may expose them.

## Validation

M3 demonstrates constraints, dynamic voxel bodies and a breakable structure; tests edits, stale collider jobs, LOD independence, mass properties, persistence and stability under load. Measure both steady simulation and collider/fracture preparation costs. Any Jolt adoption must record the major advantage over viable Rust alternatives, including total binding/copy/build/maintenance costs; a marginal isolated speedup is insufficient.

## References

[Rapier](https://github.com/dimforge/rapier), [Jolt source and platform support](https://github.com/jrouwe/JoltPhysics), [world data](0005-voxel-world-data.md), [architecture](../ARCHITECTURE.md).

## v0.2 implementation evidence

Rapier 0.32.0 is the selected solver for the bounded M3 slice under ADR-0014.
See the [physics crate](../../crates/matterweave-physics/README.md) for component
assessment, fixed-step/catch-up, interpolation, mass and inertia, spring grab,
fracture caps, collision publication and snapshot validation contracts. The full
ADR remains proposed until larger workloads and native stress gates are met.
There is no camera-dependent collision LOD; evicted distant objects are frozen
before their supporting terrain is unloaded. No Jolt integration was justified.

## Background detail preparation — 2026-09-08

Immutable source snapshots now feed a bounded single worker; opaque scene tokens
reject stale results before owner-thread publication changes any live collider.
Reset and rapid source reversals have deterministic queue regressions. A direct
ARM64 Android executable checks floor creation, standing contact, removal and
falling on OnePlus13. This is real native physics execution, while APK frame-loop
publication budgets and sustained stress remain open. See
[controller and evidence](../performance/async-detail-collision.md). ADR remains Proposed.
