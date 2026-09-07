# ADR-0005: Sparse world data and versioned derived representations

- Date: 2026-09-07
- Status: Proposed
- Basis: Engineering proposal for editable fine-detail worlds.
- Requirements: R02, R06, R12

## Context

Fine uniform voxels across an entire world can create impractical storage and update costs. Rendering, collision and lighting may need different resolutions, and asynchronous edits can otherwise publish stale data.

## Decision

Prototype compact sparse voxel blocks with explicit material identities, chunk/object IDs and revisions. Separate static terrain and movable object volumes. Keep a straightforward reference query representation. Choose occupancy/density/SDF encoding, hierarchy, block size and compression from measured cases rather than hard-coding an assumed optimal format.

Treat world/edit state as authoritative. Meshes, traversal acceleration, collision proxies, LOD parents and lighting caches are derived. Publish asynchronous results only against valid revisions; bound pending work and retired resources. Version persistent formats and generators.

## Alternatives

Dense grids are acceptable for small fixtures/reference tests. Octrees, wide trees, sparse page tables and DAGs offer different traversal/compression/edit costs. Surface-only data may serve rendering but must preserve required voxel semantics.

## Consequences

Versioning and invalidation add complexity but enable safe streaming and edits. Boundary normals/meshes, lighting visibility changes and parent LOD updates extend beyond the edited cell. Stable persistence cannot depend on transient GPU addresses.

## Validation

M1 tests coordinates, occupancy/material queries, edits and serialization against a reference. M2 compares occupancy patterns, memory, traversal, upload and edit cost. M3 stresses stale jobs, eviction and collision publication. No block dimensions or compression ratio are selected yet.

## References

[Architecture](../ARCHITECTURE.md), [rendering selection](0006-rendering-path-selection.md), [benchmarks](../BENCHMARKS.md).
