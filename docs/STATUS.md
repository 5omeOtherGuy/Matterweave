# Current status

Updated: 2026-09-07

## First Android MVP implemented

M0/M1 now have working code and build/host evidence. The ARM64 development APK
builds, with native-library alignment and signing verified. The shared app/core/
ash renderer run on host Vulkan. **Physical Android execution, lifecycle behavior
and mobile performance remain unverified.** No physical device or emulator is
attached to this environment. M2–M6 are not complete or claimed.

Implemented behavior:

- Sparse authoritative 16³ voxel chunks, seeded original terrain, material edits,
  checked revisions, reference DDA rays and exposed-face mesh generation.
- Bounded versioned save/load with atomic replacement, validation, failed-save
  preservation, and app-level corrupt-save recovery without overwriting originals.
- Direct ash/Vulkan 1.1 rendering with shaded surfaces, fog, depth and diagnostics.
  Explicit resource ownership, frame fence and per-image presentation semaphores.
- Android NativeActivity, touch movement/look/edit, elevation controls, adjustable
  movement layout, lifecycle release/recreation paths and private autosaves.
- Native host sample and repeatable smoke exercise; pinned Rust/Android build,
  dependency provenance/checksums and host/APK CI definitions.

## Verification actually executed

See [the evidence report](evidence/2026-09-07-mvp.md) and
[raw host smoke output](evidence/2026-09-07-host-smoke.log).

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS. |
| `cargo test --workspace --locked` | PASS: 14 core + 10 explorer tests, no failures. |
| Core LLVM source coverage | 94.40% lines, 94.12% regions, 95% functions; no branch-coverage claim. Reproduction in core README. |
| Native host smoke, 30 presented frames | PASS on llvmpipe with Vulkan validation enabled: aimed edits/save/reload, actual resize event, host window/renderer recreation. No validation warnings/errors. |
| Native screenshot inspection | PASS: terrain/landmarks, correctly oriented HUD and controls visible. Host software Vulkan only. |
| `android/gradlew -p android :app:assembleDebug --no-daemon` | PASS, including subsequent normal checksum-enforced build. |
| APK signing / manifest / ARM64 exports | PASS: v2 debug signature, API 28/35, ARM64, NativeActivity entry exports and expected Android system dependencies. |
| ELF/ZIP page alignment + official zipalign | PASS: packaged library LOAD segments and uncompressed ZIP entry aligned to 16384 bytes. |
| Fresh-checkout packaging | PASS at `2cd6435`: detached checkout, empty Cargo build directory; all 33 Gradle tasks executed. APK hash matches the working-checkout build. |
| Documentation integrity | PASS; rerun after each final documentation update. |
| Physical Android, emulator, thermal/GPU benchmarks | NOT RUN: no available device/emulator. No mobile timing claims. |
| GitHub Actions | Remote host/Android/docs checks run on [PR #1](https://github.com/5omeOtherGuy/Matterweave/pull/1); consult its live checks for the final integration result. |

Build artifact: `android/app/build/outputs/apk/debug/app-debug.apk` (ignored in Git).
Local deliverable: `artifacts/matterweave-arm64-debug.apk` (2.6 MB), with adjacent
`.sha256` file. [Artifact manifest](evidence/2026-09-07-apk.json) records exact hash,
size and source revision. CI retains its own artifact after a workflow run.
[Host screenshot](evidence/2026-09-07-host.png) is supporting visual evidence.
The isolated verification checkout and generated target were removed after use;
shared SDK/dependency caches and the active checkout build target were preserved.

## Decisions and boundaries

ADR-0015 is accepted as the implemented engineering foundation: Rust 1.96.0,
NativeActivity/winit, ash, Naga SPIR-V, NDK r28c, Gradle/AGP, ARM64 API 28/Vulkan 1.1.
ADR-0001/0002/0012/0014 remain accepted; ADR-0003/0004 remain superseded. Other
research proposals remain proposed. M1's reference mesh does not settle M2's
ray/mesh/hybrid or production-world representation decisions. Dependency choices
and alternatives are in [DEPENDENCIES](DEPENDENCIES.md).

This is a small synchronous baseline: whole-world remeshing/upload and save per
edit, one frame in flight, host-visible mesh buffers, no streaming, LOD, physics,
indirect illumination, reflections or second game sample. Camera flight can pass
through terrain. Counters distinguish frame/main-thread wall time, voxel payload,
mesh bytes and queried heap capacity; they are not GPU time, process RSS or free RAM.

Android rotation currently uses supported IDENTITY surface transform and compositor
rotation, with an explicit unsupported-profile error otherwise. Physical lifecycle,
orientation, input and driver validation remain required. Unextended Vulkan WSI
teardown uses the documented idle fallback; optional presentation-fence retirement
is future work. See the renderer README for precise unsafe/synchronization contracts.

## Next actions

1. Install the APK on an available ARM64 Vulkan 1.1 Android device and execute the
   [development guide's checklist](DEVELOPMENT.md#install-launch-and-collect-android-evidence).
   Record device/OS/driver/build/seed and every result; fix findings before declaring
   M0/M1 fully device-accepted.
2. Deliver changes through PRs and merge after checks pass, per the owner's standing authorization. The owner also requested the MVP APK on
   [GitHub Releases](https://github.com/5omeOtherGuy/Matterweave/releases); development APKs are marked prerelease with explicit device-test limitations.
3. Begin M2's equivalent-scene ray/mesh/hybrid and mesher comparisons with the M1
   baseline preserved. Make mobile performance selections only with device evidence.
4. Proceed to Rust physics, streaming and lighting according to the roadmap once
   their integration/measurement prerequisites are met.

Project licensing, release signing ownership and store publication remain owner
decisions. Nothing in this MVP chooses a project license or publishes a store build.
