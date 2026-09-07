# ADR-0007: Virtualized detail and bounded residency

- Date: 2026-09-07
- Status: Proposed
- Basis: Proposed implementation of the owner's accepted Nanite-like detail goal.
- Requirements: R03, R08, R12

## Context

The engine needs high detail near the camera and stable simplification as projected size decreases, without retaining every finest-resolution asset in memory. Nanite offers a relevant geometry-streaming/detail-selection reference; its exact mesh implementation is not a voxel requirement.

## Decision

Evaluate multiresolution voxel/surface representations selected by projected error, with hysteresis, bounded streaming and a coarse resident fallback. Choose an explicit transition strategy for each representation; test transitions rather than assuming blending makes them smooth. Account for projection, zoom, silhouettes, thin surfaces and openings.

Keep authoritative world and collision behavior independent of camera render LOD. Update affected parent levels after edits. Supply stable motion/history information when geometry or residency changes.

## Alternatives

Fixed uniform detail wastes work or sacrifices close fidelity. Distance-only discrete LOD is a useful baseline but may pop or handle orthographic views poorly. Adopting a Nanite-style triangle hierarchy directly may be appropriate for some derived surfaces, with edit costs measured.

## Consequences

Fine detail must exist in authored/generated source data. Geometry/material precision can be decoupled. Transition memory, asynchronous requests, cancellation and topology changes need explicit budgets and tests.

## Validation

M2/M4 record approach, retreat, zoom and rapid reversal captures; inspect seams, popping, silhouettes and thin features; measure peak residency/upload work. Physics queries must not change merely because the camera moves.

## References

[Epic Nanite overview](https://dev.epicgames.com/documentation/unreal-engine/nanite-virtualized-geometry-in-unreal-engine), [world data](0005-voxel-world-data.md), [benchmarks](../BENCHMARKS.md).
