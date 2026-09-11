# Directional shadow depth reuse

Engine implementation at `78cba4a`, 2026-09-08. Source geometry, shadow resolution,
filtering and lighting remain identical; an unchanged depth map is sampled again. This
uses the existing ash/Vulkan render pass and stored image. No new dependency or rendering
algorithm is adopted. Scope is reusable renderer efficiency, not a showcase feature or
completion of indirect illumination.

## What problem this solves

The shadow depth pass was redrawn every frame even when nothing that affects the map had
changed. Reuse skips that redraw while keeping the existing sampling path unchanged.

## How it works

- Geometry replacement or removal invalidates the map. Dynamic uploads invalidate even
  when the mesh revision is unchanged, because transformed vertices can move.
- The actual fitted light matrix is the other cache input. Sun direction, quantized camera
  movement and caster-depth bounds therefore invalidate correctly; intensity and view
  orientation alone update receiver inputs without a depth redraw.
- A new image receives a clear even when shadows are disabled. Disabled or hidden frames
  preserve valid depth; changes made during them invalidate it. Map replacement starts
  uninitialized.
- Cache publication follows successful queue submission, not command recording or
  swapchain acquisition. A presentation retry after submission retains the submitted
  depth. One frame fence protects resource writes and replacement.

The pass already stores depth in `DEPTH_STENCIL_READ_ONLY_OPTIMAL` and makes depth writes
visible to fragment sampling through its external dependency. Reuse adds no writes or
transitions. This follows the existing Vulkan synchronization model; see
[Khronos synchronization](https://docs.vulkan.org/spec/latest/chapters/synchronization.html).
Future animated caster deformation or alpha-cutout depth shaders must extend the
invalidation contract; current shadow vertices depend only on uploaded positions and
instance transforms.

`Renderer::shadow_map_updated` reports actual submitted depth updates, and GPU timings
carry the same flag. Reuse keeps shadow sampling enabled but reports no shadow-pass
duration, so it must not be counted as a new near-zero-cost shadow pass. Existing CSV
columns retain this distinction through enabled state and absent pass duration.

## What was verified

- RED: new cache contract tests fail compilation on missing `ShadowReuse`; the native
  example independently fails on missing update diagnostics.
- GREEN: 30 renderer tests; 257 workspace tests, 3 existing ignored gates; strict
  workspace/all-target Clippy. The actual 30-frame Vulkan cache smoke and 30-frame native
  app lifecycle/physics smoke pass with no Vulkan validation errors on lavapipe.
- The cache smoke exercises stationary reuse, same-revision moving geometry, chunk
  eviction, static transform replacement, rejected updates, light/camera/map changes,
  disabled/hidden frames, resize and renderer recreation. CI now runs it.
- ARM64 debug/dev opt-level 2 APK builds in 57 s; signature and ZIP/ELF 16 KiB checks pass.
- OnePlus 13 Android 16 native run: 37 route points reached, 676 physics steps,
  11.244354892 s route wall duration. 730 CSV rows validate. Of 729 completed submissions
  with shadows enabled, 284 reused depth and 445 updated it. Cleanup passed.
- The actual Android screenshots were inspected; no pixel-equivalence or full
  temporal-quality claim is made.

Exact build, device, hashes and log locations are in the
[evidence manifest](../evidence/2026-09-08-shadow-reuse.json).

## Limits and what is open

- This is functional moving-camera evidence, not a matched performance measurement. The
  phone lacks Vulkan validation layers. Sustained heat/energy acceptance requires a
  separate matched comparison and remains open.
- Automatic LOD, GI/reflections and the remaining engine milestones are unfinished.
- The earlier P02 comparison is a different frozen build pair and does not measure this
  new cache. Its final third pair completed; see [P02 analysis](p02-analysis.md).
- Independent source reviews returned no supported finding: Pi Muse hit a 429 free-quota
  limit without review, Pi Gemini timed out without findings, and Pi Astra's source review
  covered invalidation, submission and metrics but executed no checks and inspected no
  shader bodies. The lead owns those checks and final acceptance. Raw worker runs remain
  under `engine-01`.
