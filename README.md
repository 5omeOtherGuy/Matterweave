# Matterweave

**A modular native Android voxel engine and game framework, built primarily in Rust.**

The first MVP implements an editable procedural voxel world, direct Vulkan rendering
through ash, touch exploration and private save/load. It is a foundation for the
larger engine: dynamic indirect lighting, automatic fine-detail selection, complex
physics and multiple game samples remain planned milestones.

## Current implementation

- **Core:** sparse 16³ voxel chunks, material queries, revisions, deterministic
  terrain, reference ray queries, surface extraction and bounded atomic snapshots.
- **Renderer:** ash/Vulkan 1.1, direct light, ambient shading, fog, depth and a
  diagnostic HUD. Vulkan resources and platform handles stay outside world data.
- **Explorer:** Android NativeActivity, simultaneous touch move/look/edit, free
  flight, adjustable movement layout, autosave and corrupt-save recovery.
- **Delivery:** pinned Cargo/NDK/Gradle build, ARM64 debug APK, host tests,
  native Vulkan smoke exercise and APK signing/page-alignment verification.

The APK has been built and inspected locally. Shared native behavior has been
exercised on host software Vulkan; **physical Android behavior and mobile
performance are not yet verified**. See [STATUS](docs/STATUS.md) for exact evidence,
limitations and next steps. Android remains the product platform; the desktop
executable supports development and testing.

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
