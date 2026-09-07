# MVP component and toolchain selections

Assessment date: 2026-09-07. These are implementation choices under ADR-0014,
not mobile performance findings. The narrowly patched winit source is tracked in `vendor/winit`; its upstream provenance and exact lifecycle patch are documented there.
Exact transitive revisions and checksums are in [Cargo.lock](../Cargo.lock);
[the generated inventory](dependencies.json) records source, version, checksum,
repository and upstream license for every locked Cargo package, including build
and other-platform dependencies. Regenerate with `python3 tools/dependency_report.py`.

## Runtime and shader components

| Component | Version | Upstream license | Selection |
| --- | --- | --- | --- |
| [ash](https://docs.rs/ash/0.38.0+1.3.281/ash/) | 0.38.0+1.3.281 | MIT OR Apache-2.0 | Direct Vulkan access as the first renderer, consistent with the repository proposal and owner's implementation direction. |
| [ash-window](https://docs.rs/ash-window/0.13.0/ash_window/) | 0.13.0 | MIT OR Apache-2.0 | Existing raw-window-handle surface integration; avoids bespoke Android/X11 glue. |
| [winit](https://docs.rs/winit/0.30.12/winit/) | 0.30.12 | Apache-2.0 | Shared native window/input/lifecycle integration. |
| [android-activity](https://docs.rs/android-activity/0.6.0/android_activity/) | 0.6.0 | MIT OR Apache-2.0 | NativeActivity implementation, selected consistently with winit; no second native glue layer or GameActivity dependency. |
| [raw-window-handle](https://docs.rs/raw-window-handle/0.6.2/raw_window_handle/) | 0.6.2 | MIT OR Apache-2.0 OR Zlib | Borrowed platform handle contract. |
| [naga](https://docs.rs/naga/24.0.0/naga/) | 24.0.0 | MIT OR Apache-2.0 | Build-time WGSL validation and SPIR-V generation for ash; no wgpu runtime or global shader compiler. |
| [glam](https://docs.rs/glam/0.30.9/glam/) | 0.30.9 | MIT OR Apache-2.0 | Camera/vector math. |
| [bytemuck](https://docs.rs/bytemuck/1.23.2/bytemuck/) | 1.23.2 | Zlib OR Apache-2.0 OR MIT | Checked plain-data vertex/push-constant representations. |
| [serde](https://docs.rs/serde/1.0.219/serde/) / [serde_json](https://docs.rs/serde_json/1.0.140/serde_json/) | 1.0.219 / 1.0.140 | MIT OR Apache-2.0 | Typed versioned persistence with validation. |
| [font8x8](https://docs.rs/font8x8/0.3.1/font8x8/) | 0.3.1 | MIT | Reused compact diagnostic HUD glyphs. |
| [pollster](https://docs.rs/pollster/0.4.0/pollster/) | 0.4.0 | Apache-2.0 / MIT | Small startup future executor; no frame-loop async runtime. |
| [log](https://docs.rs/log/0.4.28/log/) / [android_logger](https://docs.rs/android_logger/0.15.1/android_logger/) | 0.4.28 / 0.15.1 | MIT OR Apache-2.0 | Diagnostics and Android logcat. |

wgpu was assessed as a viable safe renderer abstraction; ash is the selected
first backend. No measured speed advantage is asserted. The surface mesh is
the M1 reference, while ADR-0006 retains the M2 ray/mesh/hybrid comparison.
[Core notes](../crates/matterweave-core/README.md) assess block-mesh and explain
the deliberately small reference implementation. [Renderer notes](../crates/matterweave-render/README.md)
record memory, synchronization and lifetime contracts. [Rapier 0.32.0](https://docs.rs/rapier3d/0.32.0/rapier3d/) (Apache-2.0) is adopted for
the M3 slice: Rust solver, controller, shape queries and spring joints without FFI.
[Physics selection notes](../crates/matterweave-physics/README.md) record alternatives
and limits; no credible major Jolt advantage required a foreign integration.
[block-mesh 0.2.0](https://docs.rs/block-mesh/0.2.0/block_mesh/) (MIT OR Apache-2.0)
supplies v0.2 greedy quads after exact surface/material equivalence tests against
our retained reference mesher. This is a geometry reduction finding, not a complete
mobile rendering-path selection.

The world fixture and material colors are original procedural content. No imported
game assets are used. Dependency licenses do not select a license for Matterweave.

## Build tools

| Tool | Pinned version | Source / rationale |
| --- | --- | --- |
| Rust/Cargo | 1.96.0, edition 2021 | [Rust target documentation](https://doc.rust-lang.org/rustc/platform-support/android.html); pinned in rust-toolchain.toml, host tests and ARM64 target. |
| Android command-line tools | 19.0, archive 13114758 | [Android tools](https://developer.android.com/studio); installer verifies SHA-256 `7ec965280a073311c339e571cd5de778b9975026cfcbe79f2b1cdcb1e15317ee`. |
| Android platform / build-tools | API 35 / 35.0.0 | Native min API 28, Vulkan 1.1, ARM64; development packaging profile. |
| Android NDK | 28.2.13676358 (r28c) | [NDK](https://developer.android.com/ndk/downloads); modern native page-size support, explicit 16 KiB linker option as well. |
| Android Gradle Plugin | 8.9.2 | [Compatibility](https://developer.android.com/build/releases/agp-8-9-0-release-notes); API 35 and Gradle 8.11.1. |
| Gradle wrapper | 8.11.1 | [Upstream source](https://github.com/gradle/gradle/tree/v8.11.1); distribution SHA-256 enforced by wrapper; Maven dependencies checksum-verified. |
| JDK | CI Temurin 21.0.8; local OpenJDK 21.0.12 | Runs Gradle/signing tooling; app has no Java bytecode. Local and CI patch levels are explicitly distinct. |
| Python | 3.11+ build helpers; 3.10+ docs validator | Standard library only. |

The Gradle wrapper is unmodified upstream code, with its [upstream license](../android/gradle/LICENSE).
Wrapper JAR SHA-256: `2db75c40782f5e8ba1fc278a5574bab070adccb2d21ca5a6e5ed840888448046`.
SDK/NDK tools retain their upstream SDK licenses. Platform-tools (`adb`) is
installation/diagnostics tooling, fetched by SDK manager and not an APK build input.
GitHub Actions are pinned to commits in the workflow. Tool/dependency caches are
not source files; no credentials or signing keystores belong in Git.
