# Bounded specular reflection in the raster renderer — 2026-09-09

Branch `eval/hy4-reflections`, base `5b90975`. This advances R09/M4 with one opt-in,
bounded, scene-responsive specular bounce in the existing Vulkan raster renderer. It is
**not** complete Lumen-like lighting, not glossy reflection, and not a mobile performance
claim.

## What problem this solves

The raster renderer had no reflective voxel surfaces. This slice adds a single specular
bounce that responds to world edits, material changes and removal of the reflected
object, without requiring hardware ray tracing.

## How it works

### Bounds

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

### Reflection convention

With `p` the shaded world position, `n` the unit shading normal and `e` the eye:

```text
incident  d = normalize(p - e)
reflected r = d - 2 * dot(d, n) * n
origin    o = p + n * SURFACE_OFFSET      (SURFACE_OFFSET = 0.001)
```

**Termination of a missing ray.** The single secondary ray is clipped to the source
volume. A ray that leaves the volume, starts outside it, starts inside a solid cell
(self-intersection), meets a zero-length interval, or exhausts the trace-step budget
terminates against the scene background/fog colour `vec3(0.16, 0.24, 0.29)` at the
volume-exit distance. That colour is also the fog target, so a miss and a fully fogged hit
agree. There is no sky model, no emissive term and no second bounce.

**Shading of the reflected hit** is `albedo * (ambient(n) + max(dot(n, sun), 0) *
sun_intensity)` with `ambient(n) = 0.28 + 0.12 * max(n.y, 0)`, matching `world.wgsl`. **No
shadow-map lookup and no indirect term are applied to the reflected hit**; that is a
recorded limitation, not an oversight.

**Which surfaces reflect.** The mirror strength is the `w` component of the reflection
palette entry for the *authoritative voxel material* at `floor(p - n * SURFACE_OFFSET)`,
read from the source volume. This keeps material identity out of the vertex format and
ties reflectivity to world data. It also means dynamic meshes, static instances and
detail geometry are neither reflective nor reflected, and that a reflective face on the
outward shell of the volume terminates as a miss.

### CPU/WGSL buffer contract

Group 0 grows from four to six bindings; the uniform grows from 128 to 176 bytes. The
layout is declared once in `crates/matterweave-render/src/shader_contract.rs` and asserted
by tests, so a WGSL edit cannot silently drift from Rust.

| Binding | Type | Payload |
| --- | --- | --- |
| 0 | uniform | `LightingUniform` (176 B): `view_proj`, `sun`, `params`, `indirect_origin`, `indirect_dimensions`, `reflection_origin`, `reflection_dimensions`, `reflection_params` |
| 1 | sampled image | shadow depth (unchanged) |
| 2 | sampler | comparison sampler (unchanged) |
| 3 | storage | indirect face cache (unchanged) |
| 4 | storage | `array<u32>` material grid, `x + dims.x * (y + dims.y * z)` |
| 5 | storage | `array<vec4<f32>>` 256 entries: rgb reflectance + mirror strength in `w` |

`reflection_dimensions.w` is the enable flag, mirroring `indirect_dimensions.w`.
`reflection_params` is `(trace step bound, surface offset, 0, 0)`. Sampled-image and
storage-buffer counts stay within the Vulkan 1.1 guaranteed minimum of four per-stage
storage buffers.

### Reuse and alternatives

`RayVolume` (`crates/matterweave-render/src/ray_reference.rs`) was the first reuse
candidate and was adopted: `ReflectionVolume` composes it for the material grid, the
`+/-8192` bound check, the 64³ cell cap and `(epoch, revision, seed)` validation,
replacing the palette with one carrying mirror strength. The packing order, validation
contract and unit tests already existed; a second packer would be a parallel
implementation with its own drift risk. The traversal in `world.wgsl` is adapted from
`ray_reference.wgsl` (slab clip, integer-plane recomputation instead of accumulated
`tDelta`, ties stepping together with the lowest axis determining the normal).

Techniques considered and why:

- **Screen-space reflection** (including Arm SSR / ASR-adjacent approaches): rejected as
  the primary path. It cannot show geometry outside the primary view, which is an explicit
  acceptance case here.
- **Hardware ray tracing / `VK_KHR_ray_query`**: not required. Adding it would make a
  mobile-extensions dependency mandatory and would not remove the need for a bounded
  source volume. ADR-0008 keeps hardware RT optional.
- **Baked cubemap / reflection probe images**: rejected. A pre-authored or baked image
  cannot respond to edits, material changes or removal of a reflected object, all of which
  are acceptance cases.
- **Planar reflection (a second render pass into a texture)**: rejected for this slice. It
  handles one mirror plane and would not satisfy "reflective voxel surfaces" generally,
  and it costs a full extra scene draw. It remains a credible later optimization for
  single-plane cases.
- **Bounded DDA against an authoritative voxel crop (adopted)**: satisfies every
  acceptance case, reuses existing packing/traversal, and keeps the cost explicit and
  bounded. Cost is per reflective fragment and is reported, not hidden.

No new dependency was added; the build still uses pinned ash 0.38.0, ash-window 0.13.0,
Naga 24.0.0, bytemuck 1.23.2 and glam 0.30.9.

### Invalidation

- Any geometry upload (`update_counters`) and any shadow-resource replacement disable
  reflection until republished.
- Publication validates `(epoch, revision, seed)` **and** re-derives an FNV-1a footprint
  digest over the volume cells, so a replaced scene at an *equal* revision is rejected.
  The digest costs one `World::get` per cell (at most 262,144) and runs only at
  publication, never per frame.
- A rejected publication disables first, so stale data can never remain visible.
- The sun is a live per-frame uniform rather than baked data, so a sun change needs no
  republication. This deliberately differs from the diffuse cache, which bakes irradiance
  and therefore keys on the sun.

## What was verified

Reproducible commands (lavapipe, `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`):

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/hy4-reflections/build
export CARGO_BUILD_JOBS=1
cargo run -p matterweave-render --locked --example reflection_validation -- \
  /mnt/bench/matterweave-dev/hy4-reflections/evidence/host
xvfb-run -a cargo run -p matterweave-render --locked --example reflection_smoke
xvfb-run -a cargo run -p matterweave-explorer --locked -- --reflection-check --save /tmp/x.json
```

- Summaries are committed under [performance/reflections](reflections/README.md); raw PPM
  captures stay under `/mnt/bench/matterweave-dev/hy4-reflections/evidence/host/`.
- `reflection_validation` compiles the shipping `world.wgsl` SPIR-V through the renderer's
  exact descriptor layout, renders one analytic mirror pixel per probe to a linear
  offscreen target and compares against an independent CPU oracle (`World::raycast` plus a
  slab clip). Result: 28 predetermined non-edge probes (14 hits, 14 misses), worst
  per-channel error 0.00193 (≈0.49/255) against a 3/255 tolerance and no rejected probes;
  64 seeded randomized scenes, 346 probes, worst 0.00195; nonreflective baseline worst
  0.00196 against a 1/255 tolerance. PPM artefacts and `manifest.json` are written to the
  evidence directory.
- Off-screen proof: with reflection disabled, deleting the reflected object changes **0**
  pixels; with reflection enabled it changes 1737 pixels. The object is outside the camera
  frustum and reachable only through the reflection.
- `reflection_smoke` drives the real `Renderer` through enabling, edit invalidation, stale
  rejection, epoch replacement, disabling, resizing and full renderer recreation with
  `MATTERWEAVE_VALIDATION=1` and an empty validation stream.
- App gate (`--reflection-check`) runs eight scripted phases and reports publication cost,
  owned bytes and frame-time percentiles per phase.
- Existing gates re-run unchanged: `ray_reference_vulkan` 30 checks / 0 failures,
  `renderer_comparison` 16 fixture runs / 0 failures ("PASS Vulkan validation: no error
  messages"), `cache_smoke` ten frames, `indirect_smoke` PASS.
- Workspace: `cargo test --workspace` 404 passed / 0 failed / 3 pre-existing ignored;
  `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo fmt --all --
  --check` clean; `python3 tools/check_docs.py` passes.

Host numbers come from llvmpipe, a software rasterizer; they are correctness and mechanism
evidence only and are never presented as device performance.

## Limits and what is open

### Device measurement, 12 September 2026

Both app gates ran on the reserved OnePlus 13 (`CPH2653`, Android 16 / SDK 36, build
`BP4A.251205.006`, Adreno 830, Vulkan 1.3.284, driver 2150760522) from installed
`dev.matterweave.explorer` 0.5.0 versionCode 5. Device unplugged, battery 55%, 24.0 °C,
thermal status 0 before and after both runs. Wall-clock frame interval is display-bound
at ~16.6 ms (60 Hz) in every phase, so cost appears in the GPU timestamp, not the
interval.

Cost mode, 120 warmup + 1000 measured frames per mode:

| Mode | GPU ms | Interval p50 / p95 / p99 ms | Owned bytes | Publications |
| --- | --- | --- | --- | --- |
| off | 4.501 | 16.588 / 17.202 / 17.624 | 0 | 0 |
| on | 11.953 | 16.597 / 17.153 / 17.836 | 50176 | 1 |

Enabling reflections costs **+7.45 ms of GPU time per frame, a 2.66x increase**, and
owns 50176 bytes (46080 material + 4096 palette). It fits a 16.667 ms period with about
4.7 ms to spare; it does not fit an 8.333 ms period, so this configuration cannot hold
120 Hz on this device. No energy or thermal claim is made: these runs are far too short
to reach equilibrium, and the battery sensor did not move.

Quality mode, 8 phases, 30 measured frames each, all phases completed:

| Phase | GPU ms | Publications | Publication cost |
| --- | --- | --- | --- |
| off | 4.485 | 0 | — |
| on | 10.690 | 1 | upload 1.427 ms, fence 0.002 ms |
| camera-moved | 10.487 | 0 | — |
| occluder-added | 11.449 | 1 | upload 0.028 ms, fence 0.002 ms |
| object-removed | 11.571 | 1 | upload 0.023 ms, fence 0.001 ms |
| sun-moved | 10.969 | 0 | — |
| scene-replaced | 11.542 | 1 | upload 0.026 ms, fence 7.974 ms |
| disabled | 3.050 | 0 | — |

Camera motion and sun motion republish nothing; edits, removals and scene replacement
each republish exactly once, which is the documented invalidation contract observed on
hardware. One publication blocked **7.974 ms on its fence** — roughly half a frame, and
three orders of magnitude above the other four. It is a single observation and its cause
is not established. The `disabled` phase measures the replaced scene, not the opening
one, so its 3.050 ms is not comparable with the 4.485 ms `off` baseline.

Reports: `/mnt/bench/matterweave-dev/device-gates/2026-09-12/`.

### Not run

- Thermal conditions under sustained load, energy per frame and behaviour past thermal
  equilibrium are unmeasured. The runs above are seconds long and prove cost per frame
  only.

### Deferred explicitly

Glossy or rough reflection, shadowed reflected hits, temporal accumulation and history,
denoising, reflective dynamic meshes or static instances, multiple bounces, streaming
unbounded worlds, planar-reflection specialization, and background preparation of the
reflection volume.
