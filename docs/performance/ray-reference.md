# Bounded unit-voxel ray reference

This is the CPU pack and fragment-traversal leaf of the M2 experiment at base
`9854723`, not a selected production path, hardware RT implementation, or measured
performance improvement. [ADR-0006](../adr/0006-rendering-path-selection.md) remains
proposed. The lead owns Vulkan pipeline integration and Android comparison.

## Reuse and scope

Reuse `World::get` for authoritative data, `World::raycast` for CPU oracle cases,
the renderer's existing sun validation, glam matrix inversion, bytemuck upload
layout, and the existing Naga 24.0.0 WGSL-to-SPIR-V VS/FS build loop. No dependency
was added. ash 0.38.0 remains the integration API. Versions/licenses remain in the
[dependency record](../DEPENDENCIES.md). Rust stdlib checked arithmetic and fallible
Vec allocation cover the packing mechanics.

The custom adapter is necessary because the existing CPU DDA cannot execute in a
fragment shader and the raster mesh is not a material-grid buffer. This narrow
WGSL translation and dense upload experiment answers that gap without a second
CPU renderer, generic scene framework, or a broad component search. Adopt/adapt
for the experiment only; primary-path selection is deferred to matched evidence.

## CPU interface

`matterweave_render::ray_reference::RayVolume::pack(&World, epoch, origin,
dimensions, [[f32; 3]; 256]) -> Result<RayVolume, String>` produces an immutable
snapshot. Origin is `[i32; 3]`, dimensions `[u32; 3]`; the box is half-open
`[origin, origin + dimensions)`. All axes are nonzero and at most128. Total cells
are at most64³, regardless of shape. Both endpoints must be within±8192;
validation uses widened/checked arithmetic before allocation. Palette RGB must
be finite linear values in0..=1, including unused entries. Packed alpha is1.
Material0 is air; all IDs1..255 are opaque solids.

Accessors expose `origin()`, `dimensions()`, `materials(): &[u32]`,
`palette(): &[[f32; 4]; 256]`, `source_epoch()`, `source_revision()`,
`memory_stats()` and `valid_for(&World, epoch)`. The index is
`x + dimensions.x * (y + dimensions.y * z)` for local unsigned coordinates.
Edits never mutate a pack. Any world revision change conservatively invalidates
it, including outside-crop edits. Validation also compares seed and caller epoch.
**The caller must change epoch on replacement, load or fork, even at equal seed
and revision; World has no unique instance token. Never reuse a live epoch.**
Palette/crop changes require a new pack and corresponding uploads. Freshness must
be checked before publication, independently of uniform construction.

World content outside the box is intentionally absent, including potential
foreground occluders. Comparison fixtures must fit the box and contain only
unit World voxels, not fine-detail meshes or mesh-only dynamic objects. Use the
same supplied palette to color the raster fixture. The mesh palette helper is
private and is deliberately neither changed nor duplicated here.

## Exact Vulkan contract

All descriptors are **set0**, fragment-visible; no push constants or vertex buffer.

| Binding | Vulkan type | Required range / layout |
| --- | --- | --- |
| 0 | UNIFORM_BUFFER | 192 bytes, `RayUniform` below |
| 1 | STORAGE_BUFFER, read-only | `4 * cell_count` bytes; runtime `array<u32>`, stride4 |
| 2 | STORAGE_BUFFER, read-only | 4096 bytes; runtime `array<vec4<f32>>`, stride16, 256 entries |

Even an all-air box has at least one material word: minimum binding1 is4 bytes,
not a zero-size/null buffer. Buffer offsets must meet device uniform/storage
alignment limits. Resource allocation, staging, barriers, in-flight lifetime,
descriptor updates, and stale-upload rejection are lead-owned. No GPU resources
are created by this module.

`RayUniform` is `repr(C, align(16))`, Pod/Zeroable, size192, alignment16:

| Field | Offset | Type / convention |
| --- | --- | --- |
| inverse_view_projection | 0 | mat4, 64 bytes, column-major |
| view_projection | 64 | mat4, 64 bytes, column-major |
| eye | 128 | vec4 f32, xyz world eye, w0 |
| origin | 144 | vec4 i32, xyz integer minimum, w0 |
| dimensions | 160 | vec4 u32, xyz cell counts, w0 |
| sun | 176 | vec4 f32, xyz normalized direction to sun, w intensity |

`pack.uniform(view_projection: glam::Mat4, eye: glam::Vec3, Sun)` checks finite
invertibility/eye and reuses existing sun validation. Supply normal right-handed
finite-near/far perspective or orthographic matrices, positive near, **0..1 depth**,
not reversed depth and not infinite-far projections. Invertibility alone cannot
validate all projection semantics; these remain caller preconditions.

`VERTEX_SPIRV` / `FRAGMENT_SPIRV` expose compiled byte slices. Convert using
`ash::util::read_spv` rather than assuming byte-slice alignment. Entry points are
`vs_main` / `fs_main`. Draw3 vertices, one instance, first vertex0, triangle-list,
culling disabled, no vertex attributes. Use ordinary positive-height Vulkan
viewport and depth0..1, enable depth test/write (LESS or LESS_OR_EQUAL as appropriate
for the shared pass). Misses discard, retaining render-pass clear color/depth.
Hits write projected hit-position `clip.z / clip.w`, not fullscreen-triangle depth.

Naga's existing ADJUST_COORDINATE_SPACE flips vertex position Y. The fullscreen
triangle interpolates its **pre-flip** clip XY as a varying. That yields the
unflipped inverse-projection input at the matching framebuffer pixel; do not flip
the uniform matrix or varying again. No viewport-size uniform is required.

## Traversal and equivalence boundaries

The fragment unprojects near and far points and traces their segment, supporting
both projection modes without treating eye as an orthographic ray origin. Slab
clipping branches on zero direction before division. A parallel ray at an upper
box face misses; at a lower face it is inside. External entry snaps only known
slab planes, then crosses all exact tied integer planes, including internal
planes on other axes. Lowest crossed axis supplies the normal. DDA steps exact
ties together; empty cells skip. Negative-coordinate indexing uses local floor,
not truncation. There is no arbitrary positional epsilon. Loop bound is
`dimensions.x + dimensions.y + dimensions.z + 1` (at most385 iterations).

For an origin already inside, floor(origin) is inspected first, even on an
integer plane while moving negatively; an occupied initial cell gets zero
normal, matching World DDA. It receives ambient0.28, rather than normalizing a
zero vector. Subsequent hits use axis normals. Lighting matches basic world.wgsl:
ambient `0.28 + 0.12*max(n.y,0)`, unshadowed direct sun, fog coefficient0.013 toward
`[0.16,0.24,0.29]`. Shadows, indirect light and enhanced effects are absent/off.

**Not a bitwise-equivalence claim:** CPU World DDA uses f64 and accumulated plane
distances; WGSL uses f32 and recomputes distances from integer planes. Near-ties
and unprojection rounding need GPU image/depth validation. The CPU query starts
at its supplied origin and caps distance4096; this shader starts at camera near
and clips at far/AABB. Compare CPU queries with those same origins/ranges, using
fixtures/rays within4096. Crop out foreground geometry before using World as an
oracle; do not interpret its whole-world foreground hit as the cropped answer.

One intentional edge difference: the shader rejects zero-length AABB overlap
before checking cells. CPU can report a t0 hit from an exact lower box face moving
outward because its initial floor cell is solid. The oracle test explicitly
records that CPU behavior. Likewise a near plane inside a solid produces a t0
volume hit, not an exposed raster surface; keep comparison cameras in air unless
specifically testing this disclosed difference. Precision, grazing edges and
near-plane surfaces are pending validation gates, not proven equivalence.

## Memory accounting

`RayMemoryStats` reports logical material, palette, uniform and summed upload
bytes. Maximum:1,048,576 material +4096 palette +192 uniform = **1,052,864 bytes**.
Minimum:4 +4096 +192 =4292 bytes. The owned CPU pack stores material payload and
palette; a uniform is separately constructed. These figures exclude struct/Vec
metadata, allocator slack, Vulkan allocation granularity, staging copies,
descriptors and in-flight duplicates. They are payload arithmetic, not measured
resident memory, throughput or phone performance.

## Engineering log and checks

1. Read ADR0006, component selection, core ray/mesh/World, world shader and build
   contracts. Kept raster code untouched except the module export.
2. Added four contract tests first. Actual compile-time RED: missing RayVolume and
   RayUniform, not dependency/setup failure. Checkpoint `6c41e65`.
3. Implemented pack, uniform, shader and build-loop addition. Same four tests GREEN;
   Naga parsed, validated and emitted both shader entry points. Checkpoint `93794f9`.
4. Review caught an external AABB entry tying a negative internal grid plane;
   corrected initial simultaneous stepping and added an existing-World oracle
   fixture. Added negative-boundary/inside/crop/edge oracle cases without creating
   a duplicate CPU traversal. These tests do not execute WGSL.
5. Full renderer library suite:73 passed,0 failed. Existing vendored winit emits a
   function-item integer-cast warning; it was not changed. No new dependency or
   environment inspection was needed. No shader execution/coverage claim follows
   from compilation or CPU oracle tests.

Reproduce from the repository with `CARGO_BUILD_JOBS=1` and
`CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/engine-02/ray-reference-target`:

```sh
cargo test -p matterweave-render ray_reference --lib
cargo test -p matterweave-render --lib
cargo clippy -p matterweave-render --lib --tests -- -D warnings
python3 tools/check_docs.py
```

Logs are under `/mnt/bench/matterweave-dev/performance/engine-02/` with prefix
`ray-reference-`. The stopped initial worker target is retained until follow-up verification finishes.

## Native GPU probe verification — engine-03

The headless `ray_reference_vulkan` example now creates the actual Naga-compiled
Vulkan pipeline and reads color/depth for 24 probes across perspective and
orthographic projections. Fixtures cover negative bounds, thin axes, parallel
rays, ties and starts inside solid/air. CPU World ray queries provide independent
expected hit data; disclosed tie-normal conventions remain explicit.

Hy4's bounded attempt added the harness but timed out before final acceptance.
Lead reproduced 24 numeric matches on Radeon Vega 10 (RADV RAVEN), then enabled
Khronos synchronization validation and found 48 read-after-write hazards. The
RED checkpoint is `6f4ea61`. Explicit subpass-to-transfer and transfer-to-host
dependencies now pass the same 24 probes with no validation errors. Command
buffers/framebuffers are released before referenced attachments. Seven ray
contract tests, all 73 renderer library tests, and strict scoped Clippy pass.

```sh
VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation \
VK_LAYER_ENABLES=VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT \
cargo run --locked -p matterweave-render --example ray_reference_vulkan
```

Require both `24 checks, 0 failures` and no validation errors in captured output;
probe counts alone do not validate GPU synchronization. Logs:
`/mnt/bench/matterweave-dev/performance/engine-03/ray-lead-*`.
Synchronization follows [Khronos examples](https://github.com/KhronosGroup/Vulkan-Docs/wiki/Synchronization-Examples).

## Remaining gates

Physical Android operation, fullscreen image equivalence, same-quality ray/raster/
hybrid comparison, cropped-scene/edit coverage, device GPU/total cost and sustained
thermal behavior remain open. One-pixel probes are shader correctness evidence,
not an equivalent-quality renderer comparison or primary-path selection.

## Android precision regression

The initial Android run at `3f4aa5b` failed 2/24 probes on Adreno 830: an exact
orthographic upper-boundary parallel ray hit, and a perspective t=0 hit was
rejected after depth re-projection. Host RADV passed the same probes. The failed
report is retained under `engine-03/phone-ray-3f4aa5b`. Near-boundary offset probes
were added before repair to constrain any numerical tolerance. Android is not
yet accepted by this checkpoint.

The correction stabilizes an axis only when both unprojected segment endpoints
are within eight relative f32 ULPs of the same integer grid plane. It also accepts
1e-5 normalized-depth re-projection roundoff at near/far before clamping output
to [0,1]. These are disclosed numerical tolerances, not exact real-number ray
semantics. Offset probes at 1e-4 on either side of the thin-volume boundary remain
distinct. At large coordinates, the relative tolerance grows; broader full-image
quality comparisons remain required. The resulting 30 probes pass on Adreno 830.
This is a headless Vulkan executable, not APK presentation/lifecycle evidence.
