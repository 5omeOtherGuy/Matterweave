# ADR-0001: Native Android voxel engine and reusable framework

- Date: 2026-09-07
- Status: Accepted
- Basis: Explicit owner scope reset and repository handoff request.
- Requirements: R01, R02, R05, R14

## Context

The owner replaced an earlier browser technology demo with a bespoke, highly optimized native Android voxel engine/framework usable for many genres, and required the repository to carry enough context for a fresh session to implement it independently.

## Decision

Matterweave is the engine/framework product, Android the required runtime platform and voxels the meaningful world/object data; presentation may vary by game. The architecture must support exploration, roguelikes, creature-collecting RPGs, tiny worlds, 3D adventures and 2.5D games without implementing every genre immediately.

Desktop tests and tools may aid development. The browser demo, its hosting platform, engine code, scene limits and art direction are not required inputs. Keep the complete development context and decision history in this repository.

## Alternatives

Continuing a browser-only demo does not meet the revised goal. Building one fixed game would not establish the intended reusable framework. A general multi-platform engine is possible later, but Android must not become a secondary validation target.

## Consequences

Native lifecycle, touch interaction, packaging and actual-device verification are early work. Engine services and game rules need boundaries. No specific language, library or rendering algorithm is selected by this ADR.

## Validation

M0/M1 establish the native slice. M6 demonstrates two different samples sharing the engine. Fresh-session usability is checked through the handoff and reproducible instructions.

## References

[Project brief](../PROJECT_BRIEF.md), [requirements](../REQUIREMENTS.md), [roadmap](../ROADMAP.md).
