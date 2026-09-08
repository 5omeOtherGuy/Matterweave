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

## M1 implementation evidence — 2026-09-07

The host-testable reference now uses sparse 16³ byte-material chunks, checked
world revisions, bounded DDA queries, synchronous derived surface meshes and
validated versioned JSON snapshots. Tests cover negative coordinates, seams,
winding, ray edges, malformed saves, round trips and revision exhaustion. See
[core notes](../../crates/matterweave-core/README.md) and [STATUS](../STATUS.md).
This establishes the M1 reference; production representation, object volumes,
streaming and asynchronous publication remain proposed until M2/M3 evidence.

## Dense wetland implementation evidence — 2026-09-08

The detail crate now stores sparse fine voxel prototypes and authoritative
quarter-turn placements; source resolutions include6.25/12.5cm flora and25cm
terrain. Fine-source collision uses shared Rapier compounds independently of
visual meshes. The actual full-map app regression covers capsule placement,
jumping, source-cell removal, isolated edit journals and saved-pose/cell reload.
Generator2 prevents old placement IDs from silently changing the meaning of edits.
See [STATUS](../STATUS.md) for exact host/native/phone and release distinctions.
This is evidence for the experimental sparse-source path; comparative ray/mesh/
hybrid selection and full streaming/edit-latency gates remain open.
