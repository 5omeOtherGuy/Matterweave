# Open questions and working defaults

An open technical question is work to resolve, not automatically an owner blocker. M0/M1 now have an implemented baseline; current evidence and remaining subgates are in [STATUS](STATUS.md). Revisit this table when evidence changes.

| Question | Working direction | Who resolves / when |
| --- | --- | --- |
| Component reuse, adaptation or custom work? | Rust preference, qualifying reuse and necessity/significant-advantage policy are accepted; individual components remain open. | Implementation session under ADR-0014 and COMPONENT_SELECTION.md. |
| Rust toolchain and Android integration? | Rust 1.96.0, Cargo/NDK/Gradle, ash and NativeActivity/winit are adopted for M0/M1; see DEPENDENCIES.md. | Resolved baseline in accepted ADR-0015; revisit only with evidence. |
| Android/API/GPU floor? | MVP profile: ARM64, API 28, Vulkan 1.1 with queried capabilities; OnePlus 13/Android 16 tested; broader device coverage remains unverified. | Implementation session establishes provisional floor in M0; consult owner only if intended reach materially changes. |
| Reference phones and physical access? | Local ADB inventory was empty on 2026-09-07; physical smoke/driver evidence remains outstanding. | Implementation session in M0. Owner/device access needed only for unavailable physical validation or hardware acquisition. |
| Frame rate/resolution/RAM targets? | Proposed profiles and measurement protocol in BENCHMARKS.md; no fixed memory quota yet. | Implementation session proposes measured budgets in M1/M2; owner preference only for a material product tradeoff. |
| Exact voxel representation? | M1 reference uses sparse 16³ chunks and DDA; production representation and movable objects remain M2/M3 work. | Implementation session, ADR-0005 in M1/M2. |
| Ray, mesh or hybrid primary rendering? | Hybrid hypothesis with common-scene comparisons; ray traversal remains a serious candidate. | Implementation session, ADR-0006 in M2. |
| Dynamic GI implementation? | Probes/radiance cache plus selective tracing to evaluate. | Implementation session, ADR-0008 in M4. |
| Physics scope/library? | Start rigid bodies, constraints, character movement and editable collision; assess Rust candidates such as Rapier. Jolt requires major advantages. | Implementation session, ADR-0010 under accepted ADR-0014. |
| NPU workload and Rust interface? | Inventory actual access; reuse adequate bindings or create a narrow interface when necessary/significantly advantageous. Measure a useful workload. | Implementation session, ADR-0013/0014; defer honestly if unavailable or not beneficial. |
| First environment and samples? | Original procedural exploration scene, then a different 2.5D/top-down sample; earthy alien foliage is optional. | Implementation session; avoid waiting for custom art to test the engine. |
| Asset authoring and import? | Procedural fixtures first; add narrowly scoped import/conversion needed by samples. | Implementation session in M1/M6; record asset rights/provenance. |
| Scripting, ECS and editor? | Minimal data-oriented game services; no selected scripting language/ECS package or full editor. | Implementation session at demonstrated need, likely M6 or later. |
| Deterministic networking? | Seeded generation/save replay required; bit-exact networked physics not specified. | Future product decision; not an initial implementation gate. |
| Project license? | Preserve undecided status; do not choose a license for the owner. | Owner before representing Matterweave as licensed open source or arranging external reuse. Does not block original implementation. |
| Distribution/signing? | Development APK artifacts; no store deployment selected. | Owner when production publication/signing is needed. |

## Requesting owner input

Ask a concise question only when the decision cannot reasonably be resolved within delegated engineering work or would change explicit product intent. State the concrete action blocked and continue independent tasks. Do not bundle a speculative list of future decisions into a prerequisite for building the first native sample.
