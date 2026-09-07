# Matterweave

**A modular native Android voxel engine and game framework, built primarily in Rust.**

[v0.3](docs/V0.3.md) adds directional shadows, sun/detail controls, bounded
background terrain preparation and a breakable arch playground to the native
ash/Vulkan explorer. The v0.3 slice was tested on a OnePlus13 running
LineageOS23.2/Android16; see STATUS for delivery and verification progress.

## Current implementation

- **Core:** sparse 16³ chunks, deterministic terrain, exact voxel/ray queries,
  local revisions, greedy/reference meshes and bounded asynchronous preparation.
- **Physics:** Rapier fixed-step character, editable collision, voxel rigid bodies,
  spring grabbing, throwing, bounded fracture and validated snapshots.
- **Renderer:** ash/Vulkan 1.1 chunk caching/culling, dynamic objects, direct light,
  filtered dynamic shadows, ambient shading, fog and optional GPU timings.
- **Explorer:** native Android touch controls, walk/flight, terrain editing,
  resettable destruction playground and atomic world/object/camera/lighting saves.
- **Delivery:** pinned Rust/NDK/Gradle builds, ARM64 development APK, host and
  physical-device evidence, Vulkan validation and signing/page-alignment checks.

See [STATUS](docs/STATUS.md) for exact evidence and limitations. Full mobile
ray/mesh/hybrid comparisons, automatic fine-detail LOD, indirect lighting,
reflections and multiple game samples remain planned milestones. Android is the
product platform; the desktop executable supports development and testing.

## Build and run

After installing the tools in the [development guide](docs/DEVELOPMENT.md):

```sh
cargo test --workspace --locked
android/gradlew -p android :app:assembleDebug --no-daemon
python3 tools/verify_apk.py android/app/build/outputs/apk/debug/app-debug.apk
```

APK: `android/app/build/outputs/apk/debug/app-debug.apk`.
Minimum development profile: ARM64 Android API 28, Vulkan 1.1.
Installation, controls, validation and troubleshooting are in the development guide.

## Project context

1. [AGENTS.md](AGENTS.md): contributor and orchestration instructions.
2. [Status](docs/STATUS.md) and [handoff](docs/HANDOFF.md): current state and continuity.
3. [MVP scope](docs/MVP.md), [project brief](docs/PROJECT_BRIEF.md) and [requirements](docs/REQUIREMENTS.md).
4. [ADR index](docs/adr/README.md), [architecture](docs/ARCHITECTURE.md) and [roadmap](docs/ROADMAP.md).
5. [Dependencies](docs/DEPENDENCIES.md), [component selection](docs/COMPONENT_SELECTION.md)
   and [benchmark protocol](docs/BENCHMARKS.md).

Rust, efficient modularity and evidence-led reuse are accepted policy. Lumen-like
lighting and Nanite-like detail remain outcome goals, without a feature-parity
promise. The reference surface renderer does not resolve the planned ray/mesh/hybrid
comparison. No mobile speed advantage is claimed from host or emulator results.

For documentation checks: `python3 tools/check_docs.py`.

## License

The owner has not selected a project license. Public visibility is not a license
grant. Upstream dependency licenses, versions and provenance are recorded in
[the dependency inventory](docs/DEPENDENCIES.md); they do not license original
Matterweave code. The included Gradle wrapper retains its upstream license.
