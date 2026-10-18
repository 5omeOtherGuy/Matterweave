# Matterweave

Modular native Android voxel engine and game framework in Rust.

Status: pre-release. Development APKs are published as GitHub prereleases for trying
the engine; nothing is promised, and saves and APIs may change between versions.

[![CI](https://github.com/5omeOtherGuy/Matterweave/actions/workflows/native.yml/badge.svg?branch=main)](https://github.com/5omeOtherGuy/Matterweave/actions/workflows/native.yml)
[![release](https://img.shields.io/github/v/release/5omeOtherGuy/Matterweave?include_prereleases)](https://github.com/5omeOtherGuy/Matterweave/releases)

## What it does

Matterweave is a voxel engine for Android phones: editable worlds, physics, lighting
and streaming built natively on Vulkan, with sample games that exist to prove the
engine is reusable. The engine is the product; Android-native delivery is what counts.

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

Lumen-like lighting and Nanite-like detail are outcome goals, without a
feature-parity promise.

## Install

Download `matterweave-arm64-debug.apk` (and `SHA256SUMS` to check it) from the
[latest prerelease](https://github.com/5omeOtherGuy/Matterweave/releases) and install it
with USB debugging enabled:

```sh
sha256sum -c --ignore-missing SHA256SUMS
adb install -r matterweave-arm64-debug.apk
```

Prerelease APKs are signed with a development key; uninstall an APK built elsewhere
before installing one from a release.

## Usage

On the phone, launch **Matterweave** in landscape. On a Linux desktop with a Vulkan
driver, run the explorer from a checkout:

```sh
cargo run --locked -p matterweave-explorer -- --save /tmp/matterweave-world.json
cargo run --locked -p matterweave-explorer -- --landscape --save /tmp/matterweave-landscape/world.json
```

The desktop executable supports development and testing; it is not the product.

## Configuration

Desktop flags (`--landscape`, `--world`, `--clouds off|low|high`) and the developer
environment variables and marker files for devices are described in the
[development guide](docs/DEVELOPMENT.md).

## Compatibility

- Phones: ARM64 Android, API 28 or later, Vulkan 1.1. This is a development profile,
  not a store compatibility or performance guarantee.
- Build host: Linux x86_64 with Rust 1.99.1, JDK 21 and the pinned Android SDK/NDK.
- CI builds the APK and checks its page alignment and signing on every change; it
  does not run it on a device.

## Development

Set up a checkout once, then run the gate before you push:

```sh
.agents/setup
scripts/gate
```

The gate checks formatting, lints, the workspace tests, the documentation links and
secrets. A focused test:

```sh
cargo test --locked -p matterweave-core --test world
```

The [development guide](docs/DEVELOPMENT.md) covers the Android build, Vulkan smoke
runs and device checks; [ARCHITECTURE](docs/ARCHITECTURE.md),
[REQUIREMENTS](docs/REQUIREMENTS.md) and the [ADR index](docs/adr/README.md) record
the design and its decisions. See [CONTRIBUTING.md](CONTRIBUTING.md) before opening a
pull request.

## Contributing, support and security

- Contributions: [CONTRIBUTING.md](CONTRIBUTING.md).
- Bugs and feature requests: [issues](https://github.com/5omeOtherGuy/Matterweave/issues).
- Vulnerabilities: [SECURITY.md](SECURITY.md).
- Changes: [CHANGELOG.md](CHANGELOG.md).

## License

The owner has not selected a project license. Public visibility is not a license
grant. Upstream dependency licenses, versions and provenance are recorded in
[the dependency inventory](docs/dependencies.json); they do not license original
Matterweave code. The included Gradle wrapper and the patched winit in `vendor/winit`
retain their upstream licenses.
