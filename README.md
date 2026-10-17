# Matterweave

**A modular native Android voxel engine and game framework, built primarily in Rust.**

The workspace is at version 0.5.0. [v0.5.0](https://github.com/5omeOtherGuy/Matterweave/releases/tag/v0.5.0)
is the latest published prerelease: an intermediate engine delivery, not a finished
engine.

## Current implementation

- **Core:** sparse 16³ chunks, deterministic terrain, exact voxel/ray queries,
  local revisions, greedy/reference meshes and bounded asynchronous preparation.
- **Detail:** fine-scale authoritative detail volumes, prototype/instance scenes,
  bounded budgets and derived Source/Half/Quarter levels for automatic detail selection.
- **Renderer:** ash/Vulkan 1.1 chunk caching/culling, dynamic objects, direct light,
  filtered dynamic shadows, ambient shading, fog, optional GPU timings and a bounded
  diffuse indirect reference.
- **Physics:** Rapier fixed-step character, editable collision, voxel rigid bodies,
  spring grabbing, throwing, bounded fracture and validated snapshots.
- **Audio:** bounded PCM sound-effect service with a deterministic mixer and an
  Android AAudio backend; host-verified, with its reserved-device diagnostic not run.
- **Pacing:** deterministic, clock-free frame pacer used by the explorer's wetland
  frame loop; scheduling correctness only, with no throughput, power or thermal claim.
- **Explorer:** native Android touch controls, walk/flight, terrain editing,
  resettable destruction playground and atomic world/object/camera/lighting saves.
- **Landscape and world modes:** streamed voxel-stepped distance rings with aerial
  perspective, water, and a playable character on solid landscape terrain
  (`--landscape`, `--world`).
- **Sky and clouds:** a sky dome and an optional cloud pass (`--clouds off|low|high`).
- **Flora and biomes:** blade ground cover and trees that fade with distance, three
  walkable biomes and reachable landmarks, with deterministic per-voxel surface tone.
- **Mossbound:** a separate monster-catching RPG sample (`crates/matterweave-monsters`,
  `apps/monsters`) that ships as its own APK and reuses the engine.
- **Fidelity tools:** capture-reading metrics with pass/fail gates in
  [tools/fidelity](tools/fidelity/README.md).
- **Delivery:** pinned Rust/NDK/Gradle builds, ARM64 development APK, host and
  physical-device evidence, Vulkan validation and signing/page-alignment checks.

Open milestone work includes the equivalent-quality ray/mesh/hybrid comparison and
primary-path selection, full Lumen-like indirect lighting/reflections and sustained
efficiency acceptance. Android is the product platform; the desktop executable
supports development and testing.

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

1. [AGENTS.md](AGENTS.md): contributor and agent instructions.
2. [Requirements](docs/REQUIREMENTS.md).
3. [ADR index](docs/adr/README.md) and [architecture](docs/ARCHITECTURE.md).
4. [Development guide](docs/DEVELOPMENT.md) and the [dependency inventory](docs/dependencies.json).

Rust, efficient modularity and evidence-led reuse are accepted policy. Lumen-like
lighting and Nanite-like detail remain outcome goals, without a feature-parity
promise. The reference surface renderer does not resolve the planned ray/mesh/hybrid
comparison. No mobile speed advantage is claimed from host or emulator results.

For documentation checks: `python3 tools/check_docs.py`.

## License

The owner has not selected a project license. Public visibility is not a license
grant. Upstream dependency licenses, versions and provenance are recorded in
[the dependency inventory](docs/dependencies.json); they do not license original
Matterweave code. The included Gradle wrapper retains its upstream license.
