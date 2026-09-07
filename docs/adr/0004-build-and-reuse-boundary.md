# ADR-0004: Own voxel systems and reuse mature infrastructure

- Date: 2026-09-07
- Status: Superseded
- Basis: Response to the owner's reuse-versus-ground-up question; recommendation awaits integration evidence.
- Requirements: R02, R03, R11, R13

Superseded by [ADR-0014](0014-rust-modularity-and-evidence-led-reuse.md), which establishes Rust preference, explicit modularity, stricter reuse/custom-development criteria and a major-advantage condition for Jolt. The original recommendation below is historical.

## Context

The engine's distinctive work is efficient voxel worlds, detail, lighting and physical interaction on Android. Reimplementing all surrounding infrastructure would add risk, while a whole-engine fork may constrain the very systems being investigated.

## Decision

Propose owning voxel data, streaming, rendering policy, edit propagation and gameplay integration. Reuse mature libraries for platform integration, allocation, physics, audio, reconstruction and tools where useful. Initial candidates are Android game libraries, Vulkan Memory Allocator, Jolt and Arm ASR.

Evaluate Filament as a renderer/integration baseline, Godot plus Voxel Tools as a framework reference, and Unreal as a visual/mobile benchmark. VoxelHex and voxel-rs are algorithm/library references requiring their own Android and integration evaluation. No code is adopted by listing it here.

## Alternatives

A whole-engine foundation may still win if it satisfies the goals with less integration cost. A full rewrite is justified only for measured constraints. Selectively importing code may cost more than implementing a small well-understood algorithm.

## Consequences

Custom ownership gives control but no automatic speed advantage. Every adopted dependency needs an exact revision, license/provenance, patch policy and evidence. Do not assume a renderer library supplies a game framework or a voxel library supplies mobile-ready lighting/physics.

## Validation

M0 records the integration shortlist and adopts only necessary infrastructure. M2 compares relevant rendering options using equivalent scenes and total costs. Revisit if custom integration outweighs the measured benefit.

## References

[Research and engine comparison](../RESEARCH.md), [native foundation](0003-native-foundation.md).
