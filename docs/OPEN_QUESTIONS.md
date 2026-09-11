# Open questions and working defaults

An open technical question is work to resolve, not automatically an owner blocker.
M0/M1 have an implemented baseline; [STATUS](STATUS.md) holds current evidence and
remaining subgates. Revisit this table when evidence changes.

| Question | Working direction | Who resolves / when |
| --- | --- | --- |
| Component reuse, adaptation or custom work? | Rust preference, qualifying reuse and necessity/significant-advantage policy are accepted; individual components remain open. | Implementation session under ADR-0014 and COMPONENT_SELECTION.md. |
| Android/API/GPU floor? | MVP profile: ARM64, API 28, Vulkan 1.1 with queried capabilities; OnePlus 13/Android 16 tested; broader device coverage remains unverified. | Provisional floor established in M0; consult the owner only if intended reach materially changes. |
| Reference phones and physical access? | OnePlus 13 (CPH2653), Android 16 and Adreno 830 supply the current device evidence; no reference set or device floor is formally selected. | Owner selects reference devices/floor when intended device reach matters. |
| Frame rate/resolution/RAM targets? | Proposed profiles and measurement protocol in BENCHMARKS.md; no fixed memory quota yet. | Implementation session proposes measured budgets in M1/M2; owner preference only for a material product tradeoff. |
| Exact voxel representation? | M1 reference uses sparse 16³ chunks and DDA; production representation and movable objects remain M2/M3 work. | Implementation session, ADR-0005 in M1/M2. |
| Ray, mesh or hybrid primary rendering? | Hybrid hypothesis with common-scene comparisons; a bounded full-image ray/raster/hybrid comparison is in progress and primary-path selection is open. | Implementation session, ADR-0006 in M2. |
| Dynamic GI implementation? | Probes/radiance cache plus selective tracing to evaluate; a bounded diffuse indirect reference is implemented, but full GI and reflections remain open. | Implementation session, ADR-0008 in M4. |
| Physics scope/library? | Start rigid bodies, constraints, character movement and editable collision; assess Rust candidates such as Rapier. Jolt requires major advantages. | Implementation session, ADR-0010 under accepted ADR-0014. |
| NPU workload and Rust interface? | Inventory actual access; reuse adequate bindings or create a narrow interface when necessary/significantly advantageous. Measure a useful workload. | Implementation session, ADR-0013/0014; defer honestly if unavailable or not beneficial. |
| First environment and samples? | The original procedural exploration scene shipped; the wetland showcase is the current validation workload, and a materially different second sample remains M6 work. | Implementation session; avoid waiting for custom art to test the engine. |
| Asset authoring and import? | Procedural fixtures first; add narrowly scoped import/conversion needed by samples. | Implementation session in M1/M6; record asset rights/provenance. |
| Scripting, ECS and editor? | Minimal data-oriented game services; no selected scripting language/ECS package or full editor. | Implementation session at demonstrated need, likely M6 or later. |
| Deterministic networking? | Seeded generation/save replay required; bit-exact networked physics not specified. | Future product decision; not an initial implementation gate. |
| Project license? | Preserve undecided status; do not choose a license for the owner. | Owner before representing Matterweave as licensed open source or arranging external reuse. Does not block original implementation. |
| Distribution/signing? | Development APK artifacts; no store deployment selected. | Owner when production publication/signing is needed. |

## Resolved

- **Rust toolchain and Android integration:** Rust 1.96.0, Cargo/NDK/Gradle, ash and
  NativeActivity/winit are adopted and pinned in [DEPENDENCIES](DEPENDENCIES.md) under
  accepted [ADR-0015](adr/0015-rust-native-foundation.md); revisit only with evidence.

## Unverified

None: every item above is either demonstrably open or listed under Resolved.

## Requesting owner input

Ask a concise question only when the decision cannot reasonably be resolved within
delegated engineering work or would change explicit product intent. State the
concrete action blocked and continue independent tasks. Do not bundle a speculative
list of future decisions into a prerequisite for implementation.
