# ADR-0011: Reusable gameplay, generation and persistence

- Date: 2026-09-07
- Status: Proposed
- Basis: Engineering proposal for the owner's multi-genre and procedural-game goals.
- Requirements: R05, R06, R10, R12

## Context

A renderer alone does not provide the reusable framework requested. Conversely, implementing every genre system or a large editor before a native slice would delay evidence. Perspective and 2.5D games need common world services with different camera and simulation policies.

## Decision

Provide minimal reusable scene/entity identity, transforms, camera modes, action input, simulation/query hooks, generation and persistence. Keep turn-based combat, creature data, inventories and run rules in sample/game code. Start with original procedural fixtures and evolve an asset pipeline from actual sample needs.

Generation uses explicit seed, configuration and generator version. Persist authoritative world edits and game state independently from disposable rendering caches. Define format versions and migration/rejection behavior before saves become user data. Use action bindings for touch, controllers and host debugging.

## Alternatives

A full ECS package, scripting language, editor or plugin API may become useful but is not selected. Hard-coding the first game's rules into the engine would defeat reuse. A single universal simulation mode would poorly serve both turn-based and real-time examples.

## Consequences

Two distinct samples are needed to validate boundaries. Serialization, asset identity and coordinate conventions become early contracts. Use original or appropriately licensed content; the genre examples do not require external Pokémon assets or mechanics.

## Validation

M1 verifies seeded generation and save/load edits. M6 demonstrates two different camera/gameplay samples sharing engine services without a fork, with mobile controls and documented extension steps.

## References

[Project brief](../PROJECT_BRIEF.md), [roadmap](../ROADMAP.md), [world data](0005-voxel-world-data.md).
