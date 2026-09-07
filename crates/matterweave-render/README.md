# Direct Vulkan chunk renderer

This crate uses ash directly, as the owner requested for the first version. The
surface rasterizer now caches independently versioned chunk meshes and culls their
bounds against the camera. It remains one M2 candidate, not a completed mesh/ray
comparison or a final renderer selection.
It consumes `matterweave_core::Mesh`; gameplay and world data contain no Vulkan
handles. The app owns Android lifecycle and drops Renderer before its native
window becomes invalid, then recreates and uploads the current world on resume.

## Reused components

Pinned crates.io dependencies: ash 0.38.0 (MIT OR Apache-2.0, Vulkan binding headers
1.3.281); ash-window 0.13.0 (MIT OR Apache-2.0, platform surface creation);
raw-window-handle 0.6.2 (MIT OR Apache-2.0 OR Zlib); winit 0.30.12 (Apache-2.0, existing
Android/native window glue); Naga 24.0.0 (MIT OR Apache-2.0, build-time WGSL to
SPIR-V compiler); bytemuck 1.23.2 (MIT OR Apache-2.0 OR Zlib, explicit upload
layout); font8x8 0.3.1 (MIT, HUD glyph data). Cargo.lock records checksums.

Source/API: [ash](https://github.com/ash-rs/ash),
[ash-window](https://github.com/ash-rs/ash/tree/master/ash-window),
[Naga](https://github.com/gfx-rs/wgpu/tree/v24.0.0/naga),
[winit](https://github.com/rust-windowing/winit),
[raw-window-handle](https://github.com/rust-windowing/raw-window-handle),
[bytemuck](https://github.com/Lokathor/bytemuck),
[font8x8](https://github.com/saibatizoku/font8x8-rs).

Existing ash-window surface glue and Naga avoid custom platform bindings and a
separate C++ shader compiler build. wgpu was assessed as a viable alternative;
using ash follows the first-version direction and makes synchronization and
feature queries explicit. No measured speed advantage over wgpu is claimed.
A general allocator is deferred: each nonempty chunk uses two host-visible coherent
vertex/index allocations; dynamic objects share another pair, plus HUD and one
device-local depth allocation. Transactional chunk replacement temporarily retains
both old and new allocations. The app bounds residency. Larger scenes need staging,
suballocation and measured residency work. `mesh_bytes` reports allocated buffer
capacity, including spare dynamic capacity, but excludes Vulkan allocation padding,
HUD, depth and driver overhead.

## GPU and unsafe contracts

- Vulkan 1.1, graphics/present on one queue family, FIFO presentation, no optional
  device features. Surface color/alpha/extent and depth support are queried.
  IDENTITY surface transform is required: Android compositor handles display
  rotation for both world and HUD. Application pre-rotation is deferred; its
  potential compositor cost must be measured before an optimized path is chosen.
  Unsupported IDENTITY produces an explicit initialization error. See the
  [Android orientation guide](https://developer.android.com/games/optimize/vulkan-prerotation).
  Capability text reports actual API, device, raw vendor-specific driver version,
  memory heap capacities and validation enablement. Heap capacity is not free RAM.
- The instance retains the Vulkan loader and an Arc to the window. Resources retain
  the device, which retains the instance. RAII guards clean partial construction.
  Destruction orders framebuffer before views, buffers/images before memory,
  children before device, and device/surface before instance/window/loader.
- One frame is in flight. A fence completes before CPU writes, mesh replacement
  or command buffer reuse. Coherent memory needs no explicit flush. Upload errors
  retain the previous complete mesh. Lower-revision chunk/legacy meshes are ignored.
  Eviction also waits for the fence. Dynamic updates accept unchanged revisions
  because body transforms may change without voxel edits; reusable vertex/index
  buffers are both mapped successfully before either is modified. Allocation growth
  is transactional. Empty dynamic geometry retains capacity and submits no draw.
- The acquire semaphore is reused only after its submit fence completes. Each
  swapchain image has its own render-finished semaphore; reacquiring that image
  establishes that its previous presentation wait consumed the semaphore.
- The device is idled only on swapchain retirement and teardown, not each normal
  frame. This is the common unextended WSI cleanup fallback, not a formal proof of
  presentation completion: Vulkan 1.1 has no presentation fence. An optional
  swapchain-maintenance extension path is outstanding before claiming rigorous
  retirement guarantees. See [Khronos WSI guidance](https://docs.vulkan.org/guide/latest/swapchain_semaphore_reuse.html).
  A shared depth image is safe with one frame in flight and a render-pass
  dependency covering depth writes. The acquire wait covers color attachment use.
- Out-of-date/suboptimal presentation schedules recreation. Zero window extent
  defers rendering. Surface/device loss is a fatal explicit result: the app must
  drop the renderer. Other Vulkan failures propagate, not an indefinite retry.
- Vertex layouts use `repr(C)` and bytemuck Pod. Camera push constants are 80 bytes
  (below Vulkan's required 128-byte minimum), visible to vertex/fragment stages.
  Naga validates shaders during compilation and flips clip Y for Vulkan;
  the core supplies a right-handed projection with depth 0..1.
- Debug builds enable VK_LAYER_KHRONOS_validation when installed. Set
  MATTERWEAVE_VALIDATION=1 to request it in release or 0 to disable. If debug-utils
  is available, warnings/errors go to stderr. Layer absence is reported honestly.

## Limits and verification

The app uploads world-space chunk surfaces; each uploaded mesh carries its own
revision. Empty meshes keep their revision without buffer allocations. Culling uses
the actual mesh AABB against column-major Vulkan 0..1 clip planes, keeping intersecting
and numerically uncertain bounds. It does not change world data or collision.
`visible_chunks` counts nonempty visible chunks, while `resident_chunks` includes
empty cached chunks. Evicted asynchronous results must be discarded by callers;
there is no retained tombstone after `retain_chunks` removes a chunk.

`upload` remains a legacy whole-world compatibility path. A successful call clears
chunk caches; a successful `upload_chunk` clears the legacy mesh. Dynamic geometry
is independent, supplied with positions and normals already transformed to world
space. This avoids extending the existing 80-byte camera push-constant layout.

This is surface rasterization with direct sun, ambient term, distance fog,
depth and alpha HUD. No indirect illumination, shadows, virtualized detail,
GPU timing, asynchronous meshing, RT or mobile performance claim is implemented.
Host-visible mesh uploads and a single frame are correctness-first baselines.
The capability line reports queried capacities, not residency or timing.

`cargo test -p matterweave-render --locked` checks Vulkan near/far planes,
column-major translation, perspective camera-inside/behind cases and conservative
handling of invalid matrices. The isolated host validation example exercises live
chunk replacement/eviction, stale revisions, failed upload preservation, empty
chunks, dynamic buffer reuse/growth/empty geometry and in-flight teardown:

```sh
MATTERWEAVE_VALIDATION=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  xvfb-run -a cargo run -p matterweave-render --example cache_smoke --locked
```

It must print `cache_smoke: six frames passed` and produce no Vulkan validation
errors. This exercise is correctness evidence only, not a frame-time comparison.

Workspace commands and actual validation results belong to
[DEVELOPMENT](../../docs/DEVELOPMENT.md) and [STATUS](../../docs/STATUS.md).
Physical Android lifecycle, rotation, driver behavior and sustained performance
require device evidence; desktop/lavapipe runs do not establish those properties.
