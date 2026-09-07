# Architecture decision records

## Authority and lifecycle

**Accepted** means a governing decision with an explicit basis. ADR-0001/0002 capture owner goals; ADR-0012 establishes the setup's engineering/evidence convention. **Proposed** means a concrete hypothesis or starting recommendation, permitted to prototype but not a proven selection. **Rejected** records a considered option not adopted. **Superseded** preserves history and links to a replacement.

The owner did not approve specific libraries or a rendering algorithm in the planning discussion. Do not silently convert those recommendations into owner mandates. The implementation session may accept, revise or reject engineering proposals after their decision gates, recording evidence and who/what informed the decision. No additional owner confirmation is required for routine choices within the accepted goals. Changing product intent requires owner direction.

For an accepted ADR, append implementation/evidence notes without rewriting its historical rationale. Create a superseding ADR for a material reversal, and update this index, requirement mapping and affected documents. Use the [template](template.md); include date, status, basis, requirements, context, decision, alternatives, consequences and validation. An accepted design does not mean it has been implemented or benchmarked.

## Index

| ADR | Title | Status | Decision gate |
| --- | --- | --- | --- |
| [0001](0001-product-and-platform.md) | Native Android voxel engine and reusable framework | Accepted | Owner scope; implementation evidence through M6. |
| [0002](0002-fidelity-and-hardware-efficiency.md) | Visual ambition and sustained hardware efficiency | Accepted | Owner goals; budgets remain measured working choices. |
| [0003](0003-native-foundation.md) | Native language, platform and graphics foundation | Proposed | M0 reproducible native build and integration checks. |
| [0004](0004-build-and-reuse-boundary.md) | Own voxel systems and reuse mature infrastructure | Proposed | M0 integration review; M2 rendering comparison. |
| [0005](0005-voxel-world-data.md) | Sparse world data and versioned derived representations | Proposed | M1 correctness and M2 representation measurements. |
| [0006](0006-rendering-path-selection.md) | Compare voxel ray, mesh and hybrid rendering | Proposed | M2 equivalent-quality device comparisons. |
| [0007](0007-virtualized-detail.md) | Virtualized detail and bounded residency | Proposed | M2/M4 visual stability and streaming evidence. |
| [0008](0008-dynamic-lighting.md) | Dynamic indirect illumination and reflections | Proposed | M4 dynamic quality/cost tests. |
| [0009](0009-reconstruction-and-scalable-effects.md) | Temporal reconstruction and scalable effects | Proposed | M4 reconstruction correctness and M5 total-cost comparisons. |
| [0010](0010-physics-and-editing.md) | Physics, destruction and editable collision | Proposed | M3 physical interaction, correctness and update-cost tests. |
| [0011](0011-game-framework-and-procedural-content.md) | Reusable gameplay, generation and persistence | Proposed | M1 round trips and M6 two-sample reuse. |
| [0012](0012-validation-and-continuity.md) | Reproducible evidence and repository continuity | Accepted | Engineering convention; applies to every milestone. |
| [0013](0013-advanced-hardware-and-research.md) | Capability-based acceleration and frontier research | Proposed | Inventory in M0; workload-specific experiment before adoption. |

## Current decision order

Resolve 0003/0004 enough to build M0, then 0005 enough for the M1 correctness slice. Use 0006/0007 for focused M2 experiments. Subsequent lighting/physics proposals should not delay the native baseline. If hardware is absent, record a provisional engineering choice without inventing mobile performance evidence; continue independent development.
