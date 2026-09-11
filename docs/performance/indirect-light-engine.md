# First diffuse indirect engine slice — 2026-09-08

A provisional CPU reference producer for one-bounce diffuse indirect light, built to
establish the integral and edit-response contract for R09. It is not a claim of
superiority over DDGI and does not complete R09.

## What problem this solves

The renderer had no indirect-light producer. This slice adds a bounded, edit-responsive
diffuse term that can be computed without GPU ray tracing.

The primary [RTXGI DDGI algorithm reference](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/docs/Algorithms.md),
[SDK 1.3 README](https://github.com/NVIDIAGameWorks/RTXGI-DDGI/blob/main/README.md) and
[Lumen technical details](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine)
were inspected before algorithm implementation, and the retrieved text is retained with
the host logs. DDGI maintains irradiance **and distance** with statistical occlusion; its
documented low-frequency leakage, residency and temporal-latency risks apply here. The
SDK implementation requires GPU ray tracing with C++/HLSL/DXC integration, so it is not a
thin adapter for this CPU/no-RT baseline. Lumen's surface-cache concept is useful, but its
full engine/card/tracing integration is not a reusable drop-in. No vendor implementation
or licensed source was copied or adopted.

## How it works

The bounded surface-cache idea is adapted as one irradiance sample per exposed
authoritative unit voxel face. There is no DDGI interpolation between faces. Reused
components: World DDA/mesh, glam 0.30.9 vector bases, std `reverse_bits` for Hammersley
quadrature, existing ash 0.38.0 buffers/fences and Naga 24.0.0 WGSL compilation. No stdlib
or existing project GI producer supplies this integral/edit contract, and a thin wrapper
over DDGI cannot remove its GPU-RT implementation requirement.

Sources: `crates/matterweave-render/src/indirect.rs` (producer/math/caps),
`indirect_tests.rs` and `indirect_edge_tests.rs` (contracts), `shadow.rs`
(fence-serialized storage lifetime/validity), `world.wgsl::indirect_diffuse` (GPU lookup),
`lib.rs::upload_indirect` and `examples/indirect_smoke.rs`.

### Mathematical contract

Cosine-weighted hemisphere quadrature gathers the first visible surface's directly sunlit
Lambertian radiance. Both receiver→bounce and bounce→sun segments use authoritative DDA.
With the existing renderer's sun intensity convention `E_sun/pi`, the cached value is
`E_indirect/pi = mean(rho_hit * intensity * max(dot(n_hit, sun), 0) * visibility)`; the
shader multiplies by receiver linear albedo. Misses contribute zero. Existing baseline
ambient remains separate and unchanged. There is no sky, emissive lighting, recursive
bounce or reflection.

### Update bounds and invalidation

- At most 24,576 face slots (393,216 payload bytes), 1–256 samples per face, trace range
  0.001–256 world units, coordinate bounds ±8192. Validation runs before fallible CPU
  allocation. There is no world clone and no worker queue. The palette is fixed, finite,
  linear [0,1] and must match rendered material reflectance.
- Each update clamps independently to 16,384 DDA calls and 16,384 work iterations; two
  calls are reserved per sample, so budgets under two leave exposed-face progress paused.
  Each DDA segment itself has a finite range. Revision invalidation additionally clears at
  most the entire 393,216-byte payload outside the iteration budget. Completed faces
  publish atomically; partial faces remain black.
- Every World revision or caller replacement epoch, normalized sun direction or intensity
  change discards all partial and completed CPU data. The caller must update after edits
  (zero budget allowed) and pass the **current** World/epoch when publishing. No
  asynchronous stale jobs exist. Epoch changes identify replaced worlds even at equal
  revision.
- Geometry uploads disable GPU GI; a stale upload is rejected and disables previous
  output. Sun changes and shadow-resource recreation disable it until republished. Call
  `disable_indirect` immediately if authority changes before its render geometry can be
  uploaded.
- GPU buffer growth/replacement and writes wait the existing frame fence. Steady updates
  reuse capacity; peak replacement is bounded by two payload buffers plus Vulkan allocator
  overhead, not an unbounded queue. Disabled default uses one 16-byte dummy buffer. The
  storage read does one bounded face lookup per fragment.
- Source sampling never changes world or collision. Dynamic mesh-only objects and static
  instance scenes reject cache upload: this path cannot query those sources.

## What was verified

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

Host results:

- RED checkpoint `da926fa`: five independent contract tests failed to compile for the
  intended missing `crate::indirect` producer. GREEN `cfba1a9`: all five pass. Native
  integration/final code `697e5c0`: eight indirect tests, a Vulkan storage buffer sampled
  by the actual world fragment shader, and a runnable phased example. All work is on
  `codex/engine-indirect-light`, base `0c68565`, with no app/core/detail edits, new
  dependencies, Android execution or remote mutations.
- 77 scoped tests pass (39 core, 38 renderer); strict scoped Clippy and fmt pass. The docs
  checker passes: 133 Markdown files, 284 local links, 15 ADRs and 20 requirements; the
  diff whitespace check passes. The existing dependency-only winit function-cast warning
  is unchanged.
- Native GI: 36 phase frames plus a light-invalidation frame pass; shadow regression: 30
  frames pass. Both report validation enabled and **zero Vulkan validation messages**.
- Host: Linux 7.0.0-31-generic x86_64; llvmpipe LLVM 20.1.2 256-bit, Vulkan 1.4.318,
  driver 104865800, vendor 10005/device 0000. Cargo dev opt-level 2/debug 0. Fixture seed
  9, 7×5×7 bounds, 64 samples, 32-unit rays, 640×480, fixed camera in the example. These
  are software Vulkan correctness observations, not mobile performance.

Pixel evidence, from native xwd captures converted with
`ffmpeg -v error -y -i phase-N.xwd phase-N.png` and Pillow ImageChops/ImageStat
comparison recorded in `pixel-check.json` with PNG SHA-256s:

- Open off→on changes 181,723 pixels, mean absolute RGB difference
  [21.9612, 1.6637, 0.6223] on the 0–255 output scale: actual red bounce reaches the
  shader, not merely a CPU counter. Moving sun changes 271,128 pixels; closing roof
  changes 307,200. Reopening reproduces the original on image exactly; final off exactly
  reproduces the original off. CPU inward-wall sample open [0.253125,0.014063,0.005625],
  closed [0,0,0]. Phase-1 image was visually inspected. No quality score is inferred.
- The example completes each phase in bounded repeated calls synchronously
  (18,298–25,790 total DDA calls), not a tested mobile per-frame schedule.

Artifacts/logs remain under `/mnt/bench/matterweave-dev/performance/engine-02/`:
`indirect-red.log`, `indirect-green.log`, `indirect-final-tests.log`,
`indirect-clippy.log`, `indirect-final-build.log`, `indirect-native-final.log`,
`indirect-shadow-regression.log` and `indirect-captures/`. The exclusive build target is
removed after verification; logs/images are retained for lead handoff.

### Device evidence — integrated opt-in check

Integrated into engine-systems; opt-in Android/host app check at `7211b7d`. APK SHA256
`6bf8ab4df45dfef78d5a6489761e11235c98043382cbc2c460b1272063db72e1`, built in 67 seconds
with Cargo dev/opt-level 2/debug 0, passed ARM64, 16 KiB alignment and signature checks.
The published 0.4.0 APK predates this implementation.

OnePlus 13 CPH2653, Android 16, Adreno 830 Vulkan 1.3.284, driver 2150760522: all six
120-presented-frame phases and the final light-invalidation frame pass. A second run
passes HOME/resume: renderer released/recreated, current CPU cache republished, later
enclosure edits still correct. Device validation layers are disabled; the host 37-frame
check passes with Vulkan validation enabled.

Gray walls visibly receive red bounce. The closed enclosure sample is zero. The reopened
and original lit screenshots are pixel-identical on this fixed fixture; clean off
screenshots across the two runs are also identical. The first off capture has a WAKEUP
overlay, and a stale previous report caused an invalid phase 5 capture at the second
launch; those two captures are explicitly excluded from pixel-equivalence evidence and
the raw images are retained. This is fixed-fixture functional evidence, not general
temporal-quality acceptance.

First-run complete-phase CPU preparation plus upload was 38.908–44.686 ms; second-run
fresh phases 38.937–45.234 ms, while renderer-recreation republish was 0.125 ms with zero
traced rays. These observations show the synchronous reference can exceed a 60 Hz frame
budget. They are not a steady-state GPU/energy/thermal benchmark or a performance
advantage. Production scheduling and quality remain open.

Manifest and hashes: [Android evidence](../evidence/2026-09-08-indirect-engine.json). Raw
APK/reports/screenshots: `performance/engine-02/phone-indirect` under the shared
`/mnt/bench/matterweave-dev` artifact root. Reproduce with the explicit one-shot check
documented in [DEVELOPMENT](../DEVELOPMENT.md); before repeating, remove the previous
**owned test report** or verify a new-run marker before collecting phases.

Independent Opus 4.8/high static review found no supported Vulkan lifetime defect. A
precision concern near coordinate extremes was not reproduced by the bounded worker
experiment before its 480-second timeout, and no corrective patch is accepted. Translation
probes reported no divergence at tested in-bounds coordinates; this does not prove all
extreme rays correct. The temporary print-only probes remain on the isolated worker
branch, and matching resident geometry remains an explicit caller contract. The review ran
no builds or tests; its detailed result is retained at `engine-02/indirect-review.md`.

## Limits and what is open

- No interpolation between faces means the tested single-voxel divider does not transfer
  light between its two sides. This is **not a general no-leak guarantee**: face-center
  lighting is piecewise constant and can miss within-face shadow edges; finite rays miss
  distant sun occluders; limited quadrature misses small openings.
- Unit voxel DDA does not cover subvoxel detail, non-axis faces, mesh-only movement or
  transparent materials. Outside coverage is black indirect, with hard cache boundaries.
  Direct shadow-map coverage differs from CPU ray coverage.
- Updates intentionally drop old light immediately rather than blend stale light, so
  partial rebuilding can visibly pop or darken. Frequent sun motion can continually
  restart work.
- The caller must schedule budgets, republish after resource recreation and handle World
  replacement epochs. CPU time, upload/fence stalls, GPU fragment overhead, energy,
  sustained thermals, Android lifecycle and physical-device quality/cost are **not
  accepted or measured on Android**. Coverage percentage tooling was not run.
- Preflight process issue: an overly broad environment query inadvertently printed
  `CARGO_REGISTRY_TOKEN` in session tool output. The value is not repeated in this artifact
  or code. Lead should rotate that credential; no credential remediation or remote
  mutation was attempted by this leaf.
- Lead owns Android integration/execution, independent review, durable artifact
  publication, broader source adapters and full R09 acceptance. Keep the feature default
  off until those gates; this does not finish Lumen-like GI or reflections.
