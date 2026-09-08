# First diffuse indirect engine slice — 2026-09-08

## Actions

- RED checkpoint `da926fa`: five independent contract tests failed to compile for
  the intended missing `crate::indirect` producer. GREEN `cfba1a9`: all five pass.
- Native integration/final code `697e5c0`: eight indirect tests, Vulkan storage
  buffer sampled by the actual world fragment shader, and runnable phased example.
  All work is on `codex/engine-indirect-light`, base `0c68565`; no app/core/detail
  edits, new dependencies, Android execution, or remote mutations.
- Sources: `crates/matterweave-render/src/indirect.rs` (producer/math/caps),
  `indirect_tests.rs` and `indirect_edge_tests.rs` (contracts), `shadow.rs`
  (fence-serialized storage lifetime/validity), `world.wgsl::indirect_diffuse`
  (GPU lookup), `lib.rs::upload_indirect`, `examples/indirect_smoke.rs`.

## Decisions

Inspected the primary [RTXGI algorithm reference](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/Algorithms.md),
[SDK 1.3 README](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/README.md),
and [Lumen technical details](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine)
before substantial algorithm implementation. Retrieved text is retained with host
logs. DDGI maintains irradiance **and distance** with statistical occlusion;
its documented low-frequency leakage/residency/temporal latency risks apply.
The SDK implementation requires GPU ray tracing, C++/HLSL/DXC integration; it is
not a thin adapter for this CPU/no-RT baseline. Lumen's surface-cache concept is
useful, but its full engine/card/tracing integration is not a reusable drop-in.
No vendor implementation or licensed source was copied/adopted.

Adapt the bounded surface-cache idea, not DDGI interpolation: one irradiance
sample per exposed authoritative unit voxel face. Reuse World DDA/mesh, glam
0.30.9 vector bases, std `reverse_bits` for Hammersley quadrature, existing ash
0.38.0 buffers/fences and Naga 24.0.0 WGSL compilation. No stdlib or existing
project GI producer supplies this integral/edit contract; a thin wrapper over
DDGI cannot remove its GPU-RT implementation requirement. This is a provisional
CPU reference, not a claim of superiority over DDGI or completed R09.

Mathematical contract: cosine-weighted hemisphere quadrature gathers the first
visible surface's directly sunlit Lambertian radiance. Both receiver→bounce
and bounce→sun segments use authoritative DDA. With the existing renderer's sun
intensity convention `E_sun/pi`, cached `E_indirect/pi = mean(rho_hit * intensity
* max(dot(n_hit,sun),0) * visibility)`. The shader multiplies by receiver linear
albedo. Misses contribute zero; existing baseline ambient remains separate and
unchanged. No sky, emissive lighting, recursive bounces or reflections.

## Solutions and bounds

- At most 24,576 face slots (393,216 payload bytes), 1–256 samples per face,
  trace range 0.001–256 world units, coordinate bounds ±8192. Validate before
  fallible CPU allocation; no world clone/worker queue. Palette fixed, finite,
  linear [0,1], and must match rendered material reflectance.
- Each update clamps independently to 16,384 DDA calls and 16,384 work iterations.
  Reserve two calls per sample; budgets under two pause exposed-face progress.
  Each DDA segment itself has a finite range. Revision invalidation additionally
  clears at most the entire 393,216-byte payload outside the iteration budget.
  Completed faces publish atomically; partial faces remain black.
- Every World revision or caller replacement epoch, normalized sun direction or
  intensity change discards all partial/completed CPU data. Caller must update
  after edits (zero budget allowed), and pass the **current** World/epoch when
  publishing. No asynchronous stale jobs exist. Epoch changes identify replaced
  worlds even at equal revision. Geometry uploads disable GPU GI; stale upload
  rejects and disables previous output. Sun changes and shadow-resource recreation
  disable it until republished. Call `disable_indirect` immediately if authority
  changes before its render geometry can be uploaded.
- GPU buffer growth/replacement and writes wait the existing frame fence. Steady
  updates reuse capacity. Peak replacement is bounded by two payload buffers
  (plus Vulkan allocator overhead), not an unbounded queue. Disabled default uses
  one 16-byte dummy buffer. Storage read does one bounded face lookup per fragment.
- Source sampling never changes world or collision. Dynamic mesh-only objects and
  static instance scenes reject cache upload: this path cannot query those sources.

## Verification and reproduction

All Cargo invocations used the pinned Rust 1.96.0 toolchain and:

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/engine-02/indirect-target
export CARGO_BUILD_JOBS=1
cargo test --locked -p matterweave-render --lib indirect_tests # RED then GREEN
cargo test --locked -p matterweave-render -p matterweave-core
cargo clippy --locked -p matterweave-render --all-targets -- -D warnings
cargo fmt -p matterweave-render -- --check
cargo build --locked -p matterweave-render --example indirect_smoke --example shadow_cache_smoke
export WINIT_UNIX_BACKEND=x11
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
export VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation
export MW_INDIRECT_CAPTURE_DIR=/mnt/bench/matterweave-dev/performance/engine-02/indirect-captures
xvfb-run -a -s '-screen 0 640x480x24' "$CARGO_TARGET_DIR/debug/examples/indirect_smoke"
xvfb-run -a "$CARGO_TARGET_DIR/debug/examples/shadow_cache_smoke"
python3 tools/check_docs.py
git diff --check
```

Results: 77 scoped tests pass (39 core, 38 renderer); strict scoped Clippy and fmt
pass. Docs checker passes: 133 Markdown files, 284 local links, 15 ADRs and
20 requirements; diff whitespace check passes. Existing dependency-only winit
function-cast warning unchanged. Native GI
36 phase frames plus light-invalidation frame pass; shadow regression 30 frames
pass. Both report validation enabled and **zero Vulkan validation messages**.
Host: Linux 7.0.0-31-generic x86_64; llvmpipe LLVM 20.1.2 256-bit, Vulkan 1.4.318,
driver 104865800, vendor 10005/device 0000. Cargo dev opt-level 2/debug 0. Fixture
seed 9, 7×5×7 bounds, 64 samples, 32-unit rays, 640×480, fixed camera in example.
These are software Vulkan correctness observations, not mobile performance.

Artifacts/logs remain under `/mnt/bench/matterweave-dev/performance/engine-02/`:
`indirect-red.log`, `indirect-green.log`, `indirect-final-tests.log`,
`indirect-clippy.log`, `indirect-final-build.log`, `indirect-native-final.log`,
`indirect-shadow-regression.log`, and `indirect-captures/`. The exclusive build
target is removed after verification; logs/images are retained for lead handoff.

Native xwd captures converted using `ffmpeg -v error -y -i phase-N.xwd phase-N.png`.
Pillow ImageChops/ImageStat comparison is recorded in `pixel-check.json`, including
PNG SHA-256s. Open off→on changes 181,723 pixels, mean absolute RGB difference
[21.9612, 1.6637, 0.6223] on the 0–255 output scale: actual red bounce reaches the
shader, not merely a CPU counter. Moving sun changes 271,128 pixels; closing roof
changes 307,200. Reopening reproduces original on image exactly; final off exactly
reproduces original off. CPU inward-wall sample open [0.253125,0.014063,0.005625],
closed [0,0,0]. Phase-1 image was visually inspected. No quality score is inferred.

## Issues / Insights / remaining gates

Preflight process issue: an overly broad environment query inadvertently printed
`CARGO_REGISTRY_TOKEN` in session tool output. The value is not repeated in this
artifact or code. Lead should rotate that credential; no credential remediation
or remote mutation was attempted by this leaf.

No interpolation between faces means the tested single-voxel divider does not
transfer light between its two sides. This is **not a general no-leak guarantee**:
face-center lighting is piecewise constant and can miss within-face shadow edges;
finite rays miss distant sun occluders; limited quadrature misses small openings.
Unit voxel DDA does not cover subvoxel detail, non-axis faces, mesh-only movement,
or transparent materials. Outside coverage is black indirect, with hard cache
boundaries. Direct shadow-map coverage differs from CPU ray coverage.

Updates intentionally drop old light immediately rather than blend stale light;
partial rebuilding can visibly pop/darken. The example completes each phase in
bounded repeated calls synchronously (18,298–25,790 total DDA calls), not a tested
mobile per-frame schedule. Frequent sun motion can continually restart work.
Caller must schedule budgets, republish after resource recreation, and handle
World replacement epochs. CPU time, upload/fence stalls, GPU fragment overhead,
energy, sustained thermals, Android lifecycle and physical-device quality/cost
are **not accepted/measured on Android**. Coverage percentage tooling was not run.

Lead owns Android integration/execution, independent review, durable artifact
publication, broader source adapters and full R09 acceptance. Keep default off
until those gates; this does not finish Lumen-like GI or reflections.
