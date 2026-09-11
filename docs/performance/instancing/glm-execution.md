# Bounded GPU prototype instancing

## What problem this solves

Repeated static prototypes (foliage, props) were drawn as per-instance geometry. This adds
a bounded instancing path in `crates/matterweave-render` so N instances of one prototype
cost one batched draw with an instance-rate transform, without changing the legacy,
chunk or dynamic draw paths. Scope is `matterweave-render` only; `apps/explorer`, the lead
app and map collision are untouched.

## How it works

- New `src/static_scene.rs`, public
  `Renderer::replace_static_scene(&[Mesh], &[StaticInstance]) -> Result<StaticSceneStats>`
  plus `StaticSceneStats` / `static_scene_stats()`.
- Identity-instance fallback in the world and shadow pipelines for legacy, chunk and
  dynamic draws, so existing geometry renders unchanged.
- `world.wgsl` / `shadow.wgsl` apply a quarter-yaw + translation transform from vertex
  location 3 (packed vec4: xyz translation, w = quarter yaw). An instance-rate binding 1 is
  added to the world and shadow pipelines only.
- Shadow depth bounds include instanced scene bounds, and static batches are recorded
  without frustum culling so offscreen casters are retained. The 80-byte camera push
  constant and all descriptors are unchanged.

Transactional replacement:

- Host-side validation and packing (`plan_static_scene`) run before any device work. An
  invalid index, prototype, yaw > 3, NaN translation or empty-prototype instance rejects
  the whole update and retains the previous scene.
- Budget checks run before allocation: 128 MiB merged geometry, 16 MiB packed instances,
  u32 first-index and i32 vertex-offset range — all checked, with no silent casts.
- The frame fence is waited before buffer construction, the full new scene is built
  before the swap, and the retired scene is freed only after that wait.
- Prototypes pool into shared vertex/index buffers with per-prototype `first_index`,
  `index_count`, `vertex_offset`, `instance_offset` and `instance_count`; instances are
  grouped per prototype into one packed buffer; one `cmd_draw_indexed` batch is recorded
  per prototype with real `instanceCount` and `firstInstance`; whole-batch union-bounds
  frustum culling happens in the main pass only. An empty instance list clears the scene.
- `mesh_bytes` now includes pooled static geometry capacity. There is no per-frame scene
  construction.

## What was verified

- `cargo test -p matterweave-render --locked`: 27 passed, 0 failed. This includes
  packed-record ↔ shader-layout, quarter-yaw bound rotation including negative
  translations, interleaved instance grouping, invalid-update rejection, budget rejection
  with pinned limits, empty clear, the 80-byte push-constant freeze, and
  `full_showcase_scale_fits_the_pinned_budgets`: 880 prototypes ≈ 44.5 MiB source + 7470
  instances plan successfully inside the pinned budgets. Shader compilation (build.rs Naga
  WGSL→SPIR-V with validation) runs as part of that build.
- `cargo clippy -p matterweave-render --all-targets --locked -- -D warnings`: clean.
- `cargo fmt -p matterweave-render -- --check`: clean.
- `cargo run -p matterweave-render --example instancing_smoke` (two prototypes, four yaws,
  replacement, invalid retention, clear, resize, shadow batches): **NOT RUN** — no
  display/lavapipe environment was exercised in this session. The lead runs it under
  Xvfb/native per cache_smoke precedent, plus Vulkan validation.

## Limits and what is open

- No performance or device-behavior claim: nothing here was measured on Android or any
  GPU. Buffer capacities and source bytes are arithmetic, not render throughput.
- Identity-fallback visual equivalence of legacy, chunk and dynamic paths is asserted
  structurally (identity record = zero transform) and exercised only by the un-run smoke
  example.
- Per-instance culling, LOD and draw-call reduction beyond one batch per prototype are
  future work by design (first-version scope).
- All limits are pinned constants in `static_scene.rs` (`STATIC_MESH_BUDGET_BYTES`,
  `STATIC_INSTANCE_BUDGET_BYTES`).

### Integration seam

Call `renderer.replace_static_scene(&prototype_meshes, &instances)` at load or edit time.
Treat the returned
`StaticSceneStats { prototypes, instances, vertices, indices, source_bytes, allocated_bytes, batches }`
as the honest upload accounting; on `Err` the previous scene remains live.
`shadow_caster_meshes()` now counts static batches too. Next steps: run `instancing_smoke`
under Xvfb/lavapipe with validation, then native capture. The main app may migrate the
wetland flora path onto this API without duplicating combined-viewer geometry.
