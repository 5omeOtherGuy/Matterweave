# matterweave-render

Direct Vulkan (ash) exposed-surface rasterizer for Matterweave chunks and objects, with
directional shadows, optional diffuse indirect light and single-bounce reflection, an isolated
voxel ray reference and GPU timing. One M2 candidate, not a completed ray/mesh/hybrid
comparison or a final renderer selection until equivalent-quality device evidence exists
([ADR-0006](../../docs/adr/0006-rendering-path-selection.md)).

## What it provides

| Area | What it is |
| --- | --- |
| World geometry | `upload_chunk` caches independently versioned chunk meshes and culls their bounds against the camera; `upload` is the legacy whole-world path. |
| Dynamic geometry | `upload_dynamic` supplies CPU-transformed world-space vertices/normals, independent of chunks. |
| Static instancing | `replace_static_scene` pools unique prototype geometry into shared buffers plus one packed instance buffer; `update_static_instances` changes placements without rebuilding geometry. |
| HUD | `Hud` accumulates rectangles and `font8x8` text; the app owns logical pixel layout. |
| Lighting and shadows | `Sun`/`LightingSettings` and `render_with_lighting` implement direct sun, ambient, distance fog, a depth buffer and a directional shadow pass. |
| Indirect light | `IndirectVolume` is an opt-in one-bounce diffuse surface-irradiance cache; `AsyncIndirectLight` prepares it on one background worker. |
| Reflection | `ReflectionVolume` is an opt-in, bounded, single-bounce specular reflection source; nonreflective rendering is the default and is bit-identical to the pre-reflection output. |
| Ray reference | `ray_reference::RayVolume` packs a bounded unit-voxel grid for an isolated traversal prototype; it owns no GPU resources and makes no path selection. |
| Timing | `gpu_timings` reads optional graphics-queue timestamps after the existing frame fence. |

It consumes `matterweave_core::Mesh`; gameplay and world data contain no Vulkan handles. The
app owns Android lifecycle and drops `Renderer` before its native window becomes invalid, then
recreates and uploads the current world on resume.

Dependencies are pinned exactly in `Cargo.toml`. `ash` 0.38.0, `ash-window` 0.13.0,
`raw-window-handle` 0.6.2, `winit` 0.30.12, `Naga` 24.0.0, `bytemuck` 1.23.2, `font8x8` 0.3.1
and `glam` 0.30.9 supply Vulkan access, surface glue, build-time WGSL→SPIR-V compilation,
explicit upload layout, HUD glyphs and matrix math. [DEPENDENCIES](../../docs/DEPENDENCIES.md)
records source, version and upstream license; the lockfile records checksums. Existing
ash-window glue and Naga avoid custom platform bindings and a separate C++ shader compiler
build. wgpu was assessed as a viable alternative; no measured speed advantage over wgpu is
claimed.

## Public surface

| Item | Notes |
| --- | --- |
| `Renderer` | `new`; `resize`, `upload`, `upload_chunk`, `chunk_revision`, `retain_chunks`, `upload_dynamic`; `replace_static_scene`, `update_static_instances`, `static_scene_stats`; `set_world_visible`, `set_wetland_material_time`; `render`, `render_with_lighting`; `upload_indirect`, `disable_indirect`, `indirect_enabled`; `upload_reflection`, `disable_reflection`, `reflection_enabled`, `reflection_state`; `gpu_timings`, `gpu_timestamps_supported`; diagnostics methods. |
| Renderer fields | `mesh_revision`, `capabilities`, `mesh_bytes`, `visible_chunks`, `resident_chunks`. |
| `FrameResult` | `Presented`, `Retry`, `OutOfMemory`, `Fatal(String)`. |
| `Hud`, `Sun`, `LightingSettings` | HUD vertex accumulation; sun direction/intensity; `shadows` and `shadow_map_size` (1024 or 2048). |
| `StaticInstance`, `StaticSceneStats` | `prototype`, `translation`, `yaw_quarters`; honest source/allocated byte accounting. |
| `GpuTimings` | `frame_id`, `render_ms`, `shadow_ms`, `shadows`, `shadow_map_updated`, `shadow_map_size`. |
| Modules | `indirect` (`IndirectVolume`, `UpdateBudget`, `UpdateStats`, `MAX_FACE_SLOTS`, `MAX_UPDATE_RAYS`, `MAX_UPDATE_WORK`), `async_indirect` (`AsyncIndirectLight`, `AsyncIndirectConfig`, `AsyncIndirectStats`), `reflection` (`ReflectionVolume`, `MaterialTable`, `ReflectionSample`, `ReflectionMemoryStats`, trace/bound constants), `ray_reference` (`RayVolume`, `RayUniform`, `RayMemoryStats`), `shader_contract` (shader byte slices, binding indices, `LightingUniform`, `LIGHTING_UNIFORM_BYTES`). |

## Invariants and guarantees

### GPU and unsafe contracts

- Vulkan 1.1, graphics/present on one queue family, FIFO presentation, no optional device
  features. Surface color/alpha/extent and depth support are queried. IDENTITY surface
  transform is required: the Android compositor handles display rotation for world and HUD.
  Application pre-rotation is deferred; an unsupported IDENTITY produces an explicit
  initialization error. Capability text reports actual API, device, raw vendor-specific
  driver version, memory heap capacities and validation enablement. Heap capacity is not free
  RAM.
- The instance retains the Vulkan loader and an `Arc` to the window; resources retain the
  device, which retains the instance. RAII guards clean partial construction; destruction
  orders framebuffer before views, buffers/images before memory, children before device, and
  device/surface before instance/window/loader.
- One frame is in flight. A fence completes before CPU writes, mesh replacement or command
  buffer reuse. Coherent memory needs no explicit flush. Upload errors retain the previous
  complete mesh; lower-revision chunk/legacy meshes are ignored. Eviction waits for the fence.
  Dynamic updates accept unchanged revisions because body transforms may change without voxel
  edits; reusable vertex/index buffers are both mapped before either is modified. Allocation
  growth is transactional. Empty dynamic geometry retains capacity and submits no draw.
- The acquire semaphore is reused only after its submit fence completes. Each swapchain image
  has its own render-finished semaphore; reacquiring that image establishes that its previous
  presentation wait consumed the semaphore.
- The device is idled only on swapchain retirement and teardown, not each normal frame. This
  is the common unextended WSI cleanup fallback, not a formal proof of presentation
  completion: Vulkan 1.1 has no presentation fence. An optional swapchain-maintenance
  extension path is outstanding before claiming rigorous retirement guarantees. See the
  [Khronos WSI guidance](https://docs.vulkan.org/guide/latest/swapchain_semaphore_reuse.html).
  A shared depth image is safe with one frame in flight and a render-pass dependency covering
  depth writes; the acquire wait covers color attachment use.
- Out-of-date/suboptimal presentation schedules recreation. Zero window extent defers
  rendering. Surface/device loss is a fatal explicit result: the app must drop the renderer.
  Other Vulkan failures propagate, not an indefinite retry.
- Vertex layouts use `repr(C)` and bytemuck `Pod`. Camera push constants are 80 bytes (below
  Vulkan's required 128-byte minimum), visible to vertex/fragment stages. The group-0 lighting
  uniform is 176 bytes (`LIGHTING_UNIFORM_BYTES`). Naga validates shaders during compilation
  and flips clip Y for Vulkan; the core supplies a right-handed projection with depth 0..1.
- Debug builds enable `VK_LAYER_KHRONOS_validation` when installed. Set
  `MATTERWEAVE_VALIDATION=1` to request it in release or `0` to disable. If debug-utils is
  available, warnings/errors go to stderr; layer absence is reported honestly.
- No general GPU allocator: each nonempty chunk uses two host-visible coherent vertex/index
  allocations, dynamic objects share another pair, plus HUD and one device-local depth
  allocation. Transactional chunk replacement temporarily retains both old and new
  allocations; the app bounds residency. Larger scenes need staging, suballocation and measured
  residency work. `mesh_bytes` reports allocated buffer capacity, including spare dynamic
  capacity, but excludes Vulkan allocation padding, HUD, depth, shadow resources and driver
  overhead.

### Shadows

- `render_with_lighting(view_proj, eye, hud, &LightingSettings)` uses a public `Sun`
  (direction from surface toward sun, intensity) and a shadows toggle. New settings start with
  shadows on and a 1024 map; `shadow_map_size` accepts 1024 or 2048. `render` keeps shadows off
  with the previous sun/intensity. Invalid directions, nonfinite inputs, intensity outside
  0..=16, or unsupported map sizes return an explicit fatal frame result. The map size default
  is provisional; phone comparisons must establish its quality/cost, not desktop timings.
- The light projection has a fixed 128-world-unit square XY extent centered on the eye in
  light space. Its center snaps to map texels; camera rotation cannot change its basis or
  scale. Z bounds fit all uploaded mesh AABBs with padding and 16-unit quantization. Terrain,
  legacy meshes and dynamic geometry are culled independently against the light volume. The
  map is redrawn each enabled frame, so accepted edits and body movements affect shadows in
  the next render; shadows do not alter authoritative voxels or collision.
- Depth format selection requires both attachment and sampling support. A nearest comparison
  sampler and explicit 3x3 PCF avoid requiring optional linear depth filtering. Each PCF tap
  compares against the receiver plane's depth at the nearest sampled texel center, including
  the center tap's subtexel offset. The remaining contact bias is 0.025 world units, increased
  at grazing angles, then converted using the fitted depth span. Nearly parallel receivers
  (sun/normal cosine at most 0.0001) bypass shadows to avoid a singular plane calculation;
  their direct sunlight contribution is negligible. Shadow UV Y and the receiver plane
  gradient match Naga's Vulkan vertex Y adjustment. The outer map region fades to unshadowed
  sunlight. Bias/contact quality, grazing light and moving coverage edges require phone review;
  a bounded map cannot include terrain the application has not made resident. No third-party
  shader code is copied, and multi-cascade shadows remain a later quality/coverage decision.
- A dedicated RAII owner retains the depth image/view/memory, pass, framebuffer, pipeline,
  uniform buffer, descriptor set/pool/layout and comparison sampler. The lighting uniform is
  rewritten only after the existing frame fence. The pass transitions depth writes (early/late
  fragment tests) to fragment shader reads with explicit dependencies; a first shadows-off
  frame still clears/transitions the map so its layout and contents are valid before the world
  pipeline references it. Later off frames skip the pass. Changing map size transactionally
  replaces resources after the same fence, with pipeline-compatible descriptor layouts. Full
  renderer teardown waits before any shadow resources are destroyed. One depth map uses
  4/16 MiB at D32 for 1024/2048 (2/8 MiB at D16), excluding allocation padding; these bytes
  are not included in `mesh_bytes`.

### Optional indirect light and reflection

- `IndirectVolume` samples exposed unit voxel faces (no cross-wall interpolation) with
  cosine-weighted hemisphere rays over the authoritative `World`; misses are black (no sky,
  emissive or recursive bounce). Every world revision invalidates the finite volume. It
  represents unit world voxels only: detail meshes and dynamic mesh-only objects are not
  covered.
- `AsyncIndirectLight` prepares volumes on one worker from an immutable COW world snapshot,
  bounded to at most one pending snapshot, one running job and one buffered result, with
  latest-wins deduplication. The render owner alone publishes through `Renderer::upload_indirect`,
  which revalidates the `(replacement_epoch, revision, sun)` source key. A caller must pass a
  fresh `replacement_epoch` whenever it replaces the `World` instance, including replacements
  at equal revision.
- `ReflectionVolume` reuses `RayVolume` packing and its ±8192 bound, 64³ cell cap and
  `(epoch, revision, seed)` validation, with a palette whose `.w` carries per-material mirror
  strength. One ideal mirror bounce per reflective fragment: no recursion, no temporal history,
  no glossy/rough lobe, no denoiser and no hardware ray tracing. A reflected ray terminates
  against the background/fog colour at the volume exit; a reflected hit receives no shadow-map
  lookup and no further bounce. The volume covers unit world voxels only.
- The renderer owns publication: any geometry upload or shadow-resource replacement disables
  indirect and reflection until republished. A sun change needs no reflection republication
  (the sun is a live uniform) but does invalidate the diffuse cache.

### GPU timing

- `gpu_timestamps_supported()` reports whether the selected graphics queue exposes usable
  timestamp bits and period. `gpu_timings()` returns `Option<GpuTimings>` for the most recently
  completed submission, usually the preceding frame.
- Consumers must deduplicate `frame_id` and retain its recorded `shadows` and
  `shadow_map_size` when collecting comparisons. `render_ms` is the GPU timestamp interval
  covering the shadow and color passes and any swapchain-acquire semaphore stall; it excludes
  CPU execution and presentation of this submission, but can include waiting for the
  presentation engine to release an acquired image. It is not a measure of active GPU work
  alone. `shadow_ms` is the start-to-shadow-pass-end GPU interval when shadows are enabled
  and this submission executed the depth pass (including a first-use clear); it is `None` when
  shadows are disabled or the stored depth map was reused. Queue-stage overlap means it is not
  an independently additive cost or a substitute for matched off/on runs.
- Query availability is checked after the already required frame fence, with no new wait and no
  `WAIT` query flag. Reads account for the device's timestamp period and valid counter bits; a
  conservative CPU recording-to-read bound rejects samples that could span a full counter
  period, avoiding ambiguous multiple wraps. Unsupported or unavailable results remain `None`,
  never fabricated zero timings. Host lavapipe values are correctness evidence only; mobile
  performance remains for matched physical-device validation.

## Limits and what it does not do

- This is surface rasterization. Virtualized detail, asynchronous meshing, ray-traced
  shadows/reflections and a mobile performance claim are not implemented. Indirect diffuse and
  single-bounce reflection are opt-in bounded references, not production lighting.
- The ray reference is an isolated correctness prototype; no ray/mesh/hybrid comparison or
  final path selection has been performed ([ADR-0006](../../docs/adr/0006-rendering-path-selection.md)).
- World-space chunk surfaces carry their own revision. Empty meshes keep their revision
  without buffer allocations. Culling uses the actual mesh AABB against column-major Vulkan
  0..1 clip planes, keeping intersecting and numerically uncertain bounds; it does not change
  world data or collision. `visible_chunks` counts nonempty visible chunks; `resident_chunks`
  includes empty cached chunks. Evicted asynchronous results must be discarded by callers;
  there is no retained tombstone after `retain_chunks` removes a chunk.
- `upload` is a legacy whole-world compatibility path: a successful call clears chunk caches,
  and a successful `upload_chunk` clears the legacy mesh. Dynamic geometry is independent and
  supplied with positions/normals already in world space.
- Indirect and reflection cover unit `World` voxels only. `upload_indirect` rejects its upload
  while dynamic mesh-only objects or static scene geometry are present; reflection does not
  represent those surfaces (they are nonreflective). Both caches are bounded (indirect 24 576
  face slots, reflection 64³ cells) and are disabled by geometry uploads until republished.
- Presentation completion is not formally proven; the unextended WSI cleanup fallback is used.
  Capability text reports queried capacities, not residency or timing.
- Host-visible mesh uploads and a single frame are correctness-first baselines, not a
  frame-time comparison.

## How it is tested

From the repository root:

```sh
cargo test -p matterweave-render --locked
```

Inline unit tests cover Vulkan near/far planes, column-major translation, perspective
camera-inside/behind cases, conservative handling of invalid matrices, the group-0 uniform
layout and binding indices, shader module signatures, indirect and reflection validation,
ray packing, static-scene planning, lighting projection and GPU-timing arithmetic.

The isolated host validation example exercises live chunk replacement/eviction, stale
revisions, failed upload preservation, empty chunks, dynamic buffer reuse/growth/empty
geometry, off/on/off shadow toggles, off-screen casters, 1024/2048 map replacement, resize,
zero extent, full renderer recreation and in-flight teardown:

```sh
MATTERWEAVE_VALIDATION=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  xvfb-run -a cargo run -p matterweave-render --example cache_smoke --locked
```

It must print `cache_smoke: ten frames passed` and produce no Vulkan validation errors. This
exercise is correctness evidence only, not a frame-time comparison. Other examples exercise
static instancing (`instancing_smoke`), shadow reuse (`shadow_cache_smoke`), indirect and
reflection upload/publish (`indirect_smoke`, `reflection_smoke`, `reflection_validation`),
asynchronous indirect preparation (`async_indirect`) and the ray reference
(`ray_reference_vulkan`). Host lavapipe values are correctness evidence only.

Workspace commands and actual validation results belong to
[DEVELOPMENT](../../docs/DEVELOPMENT.md) and [STATUS](../../docs/STATUS.md). Physical Android
lifecycle, rotation, driver behavior and sustained performance require device evidence;
desktop/lavapipe runs do not establish those properties. Engine notes and measurements are in
[ray reference](../../docs/performance/ray-reference.md),
[indirect light](../../docs/performance/indirect-light-engine.md),
[reflections](../../docs/performance/reflections-engine.md),
[async indirect](../../docs/performance/async-indirect.md) and
[shadow reuse](../../docs/performance/shadow-reuse.md).
