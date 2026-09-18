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

Project brief, roadmap, [world data](0005-voxel-world-data.md).

## Second-sample implementation note — 2026-09-18

`apps/monsters` (Mossbound) is the second camera/gameplay sample this ADR's
validation asks for, built without forking the engine and without touching the
explorer. Its rules and content live in `crates/matterweave-monsters` with no
window, renderer, physics or Android dependency; the host composes `core`,
`render`, `physics`, `pacing` and `audio` directly and owns its own lifecycle
transitions, touch screens, saves and settings. It ships as its own Android
module (`android/game`, application id `dev.matterweave.mossbound`) so both
apps install side by side with separate private data.

The sample keeps turn-based combat, creature data, inventories and progression
in game code, as this ADR requires. Overworld maps are authored structured
data (paths, patches, buildings, exits) expanded deterministically into voxel
worlds; encounter tables, trainer teams, evolution and learnsets are validated
by tests rather than generated freely. Reachability, roster obtainability,
save versioning and the full opening-to-badge loop are covered by tests, and a
scripted host exercise drives the real render loop through one encounter,
capture and save. Physical-device playability, polish and sustained
performance remain unverified by this note.

