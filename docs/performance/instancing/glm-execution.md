# GLM execution log — bounded GPU prototype instancing

1. **Scope** — `crates/matterweave-render` only: new `src/static_scene.rs`, public
   `Renderer::replace_static_scene(&[Mesh], &[StaticInstance]) -> Result<StaticSceneStats>`
   plus `StaticSceneStats`/`static_scene_stats()`; identity-instance fallback in the
   world/shadow pipelines for legacy/chunk/dynamic draws; `world.wgsl`/`shadow.wgsl`
   quarter-yaw+translation transform at vertex location 3 (packed vec4: xyz translation,
   w = quarter yaw); instance-rate binding 1 added to world and shadow pipelines only;
   shadow depth bounds include instanced scene bounds and static batches are recorded
   without frustum culling (offscreen casters retained); 80-byte camera push constant
   unchanged; descriptors unchanged. No commits/push; `apps/explorer`, lead app and map
   collision untouched.

2. **Implemented** — Transactional replacement: host-side validation and packing
   (`plan_static_scene`) run before any device work (invalid index/prototype/yaw>3/
   NaN translation/empty-prototype-instance ⇒ whole update rejected, previous scene
   retained); budget checks before allocation (128 MiB merged geometry, 16 MiB packed
   instances, u32 first-index and i32 vertex-offset range — all checked, no silent
   casts); fence wait before buffer construction; full construction before swap; retired
   scene freed only after that wait. Prototypes pool into shared vertex/index buffers
   with per-prototype `first_index/index_count/vertex_offset/instance_offset/
   instance_count`; instances grouped per prototype into one packed buffer; one
   `cmd_draw_indexed` batch per prototype with real `instanceCount` and
   `firstInstance`; whole-batch union-bounds frustum culling in the main pass only.
   Empty instance list clears the scene. `mesh_bytes` now includes pooled static
   geometry capacity. No per-frame scene construction.

3. **Verification** — `cargo test -p matterweave-render --locked`: 27 passed, 0 failed
   (includes packed-record↔shader-layout, quarter-yaw bound rotation incl. negative
   translations, interleaved instance grouping, invalid-update rejection, budget
   rejection with pinned limits, empty clear, 80-byte push-constant freeze, and
   `full_showcase_scale_fits_the_pinned_budgets`: 880 prototypes ≈ 44.5 MiB source +
   7470 instances plan successfully inside the pinned budgets). Shader compilation
   (build.rs Naga WGSL→SPIR-V with validation) runs as part of that build.
   `cargo clippy -p matterweave-render --all-targets --locked -- -D warnings`: clean.
   `cargo fmt -p matterweave-render -- --check`: clean.
   `cargo run -p matterweave-render --example instancing_smoke` (two prototypes, four
   yaws, replacement, invalid retention, clear, resize, shadow batches): **NOT RUN** —
   no display/lavapipe environment exercised in this leaf session; lead to run under
   Xvfb/native per cache_smoke precedent, plus Vulkan validation.

4. **Limits / not claimed** — No performance or device-behavior claims: nothing here
   was measured on Android or any GPU; buffer capacities and source bytes are
   arithmetic, not render throughput. Identity-fallback visual equivalence of
   legacy/chunk/dynamic paths is asserted structurally (identity record = zero
   transform) and exercised only by the un-run smoke example. Per-instance culling,
   LOD and draw-call reduction beyond one batch per prototype are future work by
   design (first-version scope). `ivory` tower note: none — all limits are pinned
   constants in `static_scene.rs` (`STATIC_MESH_BUDGET_BYTES`, 
   `STATIC_INSTANCE_BUDGET_BYTES`).

5. **Handoff** — Lead integration seam: call
   `renderer.replace_static_scene(&prototype_meshes, &instances)` at load/edit time;
   treat the returned `StaticSceneStats { prototypes, instances, vertices, indices,
   source_bytes, allocated_bytes, batches }` as the honest upload accounting; on `Err`
   the previous scene remains live. `shadow_caster_meshes()` now counts static batches
   too. Next: run `instancing_smoke` under Xvfb/lavapipe with validation, then native
   capture; main app may migrate the wetland flora path onto this API without
   duplicating combined-viewer geometry.
