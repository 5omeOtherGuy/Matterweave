# Bounded specular reflection in the raster renderer — 2026-09-09

Branch `eval/hy4-reflections`, base `5b90975`. This advances R09/M4: one opt-in,
bounded, scene-responsive specular bounce in the existing Vulkan raster renderer.
It is **not** complete Lumen-like lighting, not glossy reflection, and not a
mobile performance claim.

## Scope and limits (experiment, not product guarantees)

| Bound | Value | Where enforced |
| --- | --- | --- |
| Source volume | at most 64³ cells (`MAX_REFLECTION_CELLS`), each axis ≤ 64 | `ReflectionVolume::pack` |
| Coordinate range | `+/-8192` endpoints | reused `RayVolume::pack` |
| Trace steps | configurable 1..=512; effective bound is `min(steps, dims.x+dims.y+dims.z+1)` | `reflection_params.x`, `trace_bound()` |
| Secondary rays | exactly one per reflective fragment, no recursion | `world.wgsl::specular_reflection` |
| Temporal history | none | — |
| Pending jobs | none; one publication at a time, one frame in flight | `Renderer::upload_reflection` |
| GPU storage | ≤ 1 MiB material grid + 4 KiB palette (`reflection_state()`) | `shadow.rs` |
| CPU resident | material grid + 256 vec4 palette + struct (`memory_stats()`) | `reflection.rs` |
| Staging | none: one host-visible coherent destination buffer per resource, rewritten in place after the frame fence | `Buffer::write` |

## Reflection convention

With `p` the shaded world position, `n` the unit shading normal and `e` the eye:

```text
incident  d = normalize(p - e)
reflected r = d - 2 * dot(d, n) * n
origin    o = p + n * SURFACE_OFFSET      (SURFACE_OFFSET = 0.001)
```

**Termination of a missing ray.** The single secondary ray is clipped to the
source volume. A ray that leaves the volume, starts outside it, starts inside a
solid cell (self-intersection), meets a zero-length interval, or exhausts the
trace-step budget terminates against the scene background/fog colour
`vec3(0.16, 0.24, 0.29)` at the volume-exit distance. That colour is also the fog
target, so a miss and a fully fogged hit agree. There is no sky model, no
emissive term and no second bounce.

**Shading of the reflected hit** is `albedo * (ambient(n) + max(dot(n, sun), 0) *
sun_intensity)` with `ambient(n) = 0.28 + 0.12 * max(n.y, 0)`, matching
`world.wgsl`. **No shadow-map lookup and no indirect term are applied to the
reflected hit**; that is a recorded limitation, not an oversight.

**Which surfaces reflect.** The mirror strength is the `w` component of the
reflection palette entry for the *authoritative voxel material* at
`floor(p - n * SURFACE_OFFSET)`, read from the source volume. This keeps material
identity out of the vertex format and ties reflectivity to world data. It also
means dynamic meshes, static instances and detail geometry are neither reflective
nor reflected, and that a reflective face on the outward shell of the volume
terminates as a miss.

## CPU/WGSL buffer contract

Group 0 grows from four to six bindings; the uniform grows from 128 to 176 bytes.
The layout is declared once in `crates/matterweave-render/src/shader_contract.rs`
and asserted by tests, so a WGSL edit cannot silently drift from Rust.

| Binding | Type | Payload |
| --- | --- | --- |
| 0 | uniform | `LightingUniform` (176 B): `view_proj`, `sun`, `params`, `indirect_origin`, `indirect_dimensions`, `reflection_origin`, `reflection_dimensions`, `reflection_params` |
| 1 | sampled image | shadow depth (unchanged) |
| 2 | sampler | comparison sampler (unchanged) |
| 3 | storage | indirect face cache (unchanged) |
| 4 | storage | `array<u32>` material grid, `x + dims.x * (y + dims.y * z)` |
| 5 | storage | `array<vec4<f32>>` 256 entries: rgb reflectance + mirror strength in `w` |

`reflection_dimensions.w` is the enable flag, mirroring `indirect_dimensions.w`.
`reflection_params` is `(trace step bound, surface offset, 0, 0)`. Sampled-image
and storage-buffer counts stay within the Vulkan 1.1 guaranteed minimum of four
per-stage storage buffers.

## Reuse and component assessment

`RayVolume` (`crates/matterweave-render/src/ray_reference.rs`) was the first reuse
candidate and was adopted: `ReflectionVolume` composes it for the material grid,
the `+/-8192` bound check, the 64³ cell cap and `(epoch, revision, seed)`
validation, replacing the palette with one carrying mirror strength. Reason: the
packing order, validation contract and unit tests already exist; a second packer
would be a parallel implementation with its own drift risk. The traversal in
`world.wgsl` is adapted from `ray_reference.wgsl` (slab clip, integer-plane
recomputation instead of accumulated `tDelta`, ties stepping together with the
lowest axis determining the normal).

Techniques considered and why:

- **Screen-space reflection** (including Arm SSR / ASR-adjacent approaches):
  rejected as the primary path. It cannot show geometry outside the primary view,
  which is an explicit acceptance case here.
- **Hardware ray tracing / `VK_KHR_ray_query`**: not required. Adding it would
  make a mobile-extensions dependency mandatory and would not remove the need for
  a bounded source volume. ADR-0008 keeps hardware RT optional.
- **Baked cubemap / reflection probe images**: rejected. A pre-authored or baked
  image cannot respond to edits, material changes or removal of a reflected
  object, all of which are acceptance cases.
- **Planar reflection (a second render pass into a texture)**: rejected for this
  slice. It handles one mirror plane and would not satisfy "reflective voxel
  surfaces" generally, and it costs a full extra scene draw. It remains a
  credible later optimization for single-plane cases.
- **Bounded DDA against an authoritative voxel crop (adopted)**: satisfies every
  acceptance case, reuses existing packing/traversal, and keeps the cost explicit
  and bounded. Cost is per reflective fragment and is reported, not hidden.

No new dependency was added; the build still uses pinned ash 0.38.0,
ash-window 0.13.0, Naga 24.0.0, bytemuck 1.23.2 and glam 0.30.9.

## Invalidation

- Any geometry upload (`update_counters`) and any shadow-resource replacement
  disable reflection until republished.
- Publication validates `(epoch, revision, seed)` **and** re-derives an FNV-1a
  footprint digest over the volume cells, so a replaced scene at an *equal*
  revision is rejected. The digest costs one `World::get` per cell (at most
  262,144) and runs only at publication, never per frame.
- A rejected publication disables first, so stale data can never remain visible.
- The sun is a live per-frame uniform rather than baked data, so a sun change
  needs no republication. This deliberately differs from the diffuse cache,
  which bakes irradiance and therefore keys on the sun.

## Validation performed on the host

Reproducible commands (lavapipe, `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`):

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/hy4-reflections/build
export CARGO_BUILD_JOBS=1
cargo run -p matterweave-render --locked --example reflection_validation -- \
  /mnt/bench/matterweave-dev/hy4-reflections/evidence/host
xvfb-run -a cargo run -p matterweave-render --locked --example reflection_smoke
xvfb-run -a cargo run -p matterweave-explorer --locked -- --reflection-check --save /tmp/x.json
```

- Summaries are committed under [performance/reflections](reflections/README.md);
  raw PPM captures stay under
  `/mnt/bench/matterweave-dev/hy4-reflections/evidence/host/`.
- `reflection_validation` compiles the shipping `world.wgsl` SPIR-V through the
  renderer's exact descriptor layout, renders one analytic mirror pixel per probe
  to a linear offscreen target and compares against an independent CPU oracle
  (`World::raycast` plus a slab clip). Result: 28 predetermined non-edge probes
  (14 hits, 14 misses), worst per-channel error 0.00193 (≈0.49/255) against a
  3/255 tolerance and no rejected probes; 64 seeded randomized scenes, 346
  probes, worst 0.00195; nonreflective baseline worst 0.00196 against a 1/255
  tolerance. PPM artefacts and `manifest.json` are written to the evidence
  directory.
- Off-screen proof: with reflection disabled, deleting the reflected object
  changes **0** pixels; with reflection enabled it changes 1737 pixels. The
  object is outside the camera frustum and reachable only through the reflection.
- `reflection_smoke` drives the real `Renderer` through enabling, edit
  invalidation, stale rejection, epoch replacement, disabling, resizing and full
  renderer recreation with `MATTERWEAVE_VALIDATION=1` and an empty validation
  stream.
- App gate (`--reflection-check`) runs eight scripted phases and reports
  publication cost, owned bytes and frame-time percentiles per phase.

Host numbers come from llvmpipe, a software rasterizer; they are correctness and
mechanism evidence only and are never presented as device performance.

## Not run

- Reserved Android device: all device gates (item 10/11 of the acceptance list)
  are **NOT RUN**. The coordinator owns the reservation; the ready-to-run
  candidate is the `--reflection-cost` / `reflection-check.txt` gate described in
  [DEVELOPMENT.md](../DEVELOPMENT.md).
- Thermal conditions, GPU timings on mobile and sustained behaviour are unmeasured.

## Deferred explicitly

Glossy/rough reflection, shadowed reflected hits, temporal accumulation and
history, denoising, reflective dynamic meshes/static instances, multiple bounces,
streaming unbounded worlds, planar-reflection specialization, and background
preparation of the reflection volume.
