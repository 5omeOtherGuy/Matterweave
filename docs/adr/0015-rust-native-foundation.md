# ADR-0015: Rust native Android foundation

- Date: 2026-09-07
- Status: Accepted
- Basis: Engineering selection under accepted ADR-0014, owner-directed ash first backend, implemented ARM64 packaging and host integration evidence.
- Requirements: R01, R03, R10, R13, R17, R18, R19

## Context

Rust is now the default language. The next session needs a concrete Android build and graphics starting point, with room for efficient hardware integration and reusable subsystems. No engine implementation exists at this decision point.

## Decision

Propose a Cargo workspace, a pinned stable Rust toolchain and committed Cargo.lock for the workspace/sample applications. Start with the aarch64-linux-android target, the Android NDK and Gradle packaging. Pin the Rust edition/MSRV and compatible Android tools in M0. Add CMake only for dependencies or tooling that actually require it. Use a small Kotlin/Java layer where Android integration benefits or requires it.

Investigate android-activity/GameActivity integration rather than hand-writing lifecycle/input glue. Follow the selected Rust integration's instructions; do not combine incompatible native glue layers. Keep a host-testable core and reproducible APK assembly.

Investigate ash for direct Vulkan control. wgpu remains a viable alternative if its exposed capabilities and performance meet the strict criteria. Missing features must be identified in the actual API/version; do not assume an abstraction is slower or prohibits all advanced features. Shader-language/compiler selection is separate from host-language selection.

Prefer existing allocation, math, concurrency and serialization libraries when they meet requirements. Evaluate Rapier for the first physics path; apply ADR-0014's major-advantage threshold before adopting Jolt. No named dependency is pinned or selected in this proposal.

## Alternatives

A C++ engine core is superseded by the owner's Rust direction. Small foreign-library/platform adapters remain allowed under the exception policy. Existing Rust frameworks or libraries may supply useful components; Rust does not imply selecting a whole framework or automatically selecting wgpu. Nightly-only language/toolchain features need a concrete necessity or significant benefit.

## Consequences

Cargo, Android packaging and any native dependencies must form one reproducible build. FFI contracts specify ownership, layout/alignment, callback threads, cancellation and panic/error handling. Batch transfers and calls where useful; test the total pipeline. Keep unsafe sections localized with documented safety invariants and applicable validation.

Logical modularity should preserve contiguous data, efficient work batching and opportunities for static dispatch. Do not clone large world data or introduce broad shared locking merely to satisfy an inconvenient ownership model; design the ownership boundary first.

## Validation

M0 produces a clean-checkout ARM64 APK with input/lifecycle checks and capability reporting, plus Rust formatting/lint/test commands as real targets appear. Validate native-library page-size compatibility and packaging. Record exact versions and selection evidence. M1/M2 resolve representation/rendering choices; a successful build does not prove mobile performance.

## References

[Accepted Rust policy](0014-rust-modularity-and-evidence-led-reuse.md), [Rust Android target](https://doc.rust-lang.org/rustc/platform-support/android.html), [ash](https://github.com/ash-rs/ash), [wgpu](https://github.com/gfx-rs/wgpu), [android-activity](https://github.com/rust-mobile/android-activity), [Rapier](https://github.com/dimforge/rapier), [Rust FFI guidance](https://doc.rust-lang.org/nomicon/ffi.html).

## Implementation decision and evidence — 2026-09-07

Accepted for the M0/M1 baseline: Rust/Cargo 1.96.0 (edition 2021), NDK
28.2.13676358, Gradle 8.11.1/AGP 8.9.2, API 28 minimum/API 35 target and ARM64.
The first backend uses ash 0.38.0 with ash-window 0.13.0. winit 0.30.12 and
android-activity 0.6.0 provide a consistent NativeActivity implementation.
No Java/Kotlin app source, GameActivity glue, CMake or physics library is required.
Naga 24.0.0 validates WGSL and generates SPIR-V at build time. Dependency locks,
checksums, licenses and alternatives are in [DEPENDENCIES](../DEPENDENCIES.md).

The development APK builds; native ARM64 exports, system-library dependencies,
signing and 16 KiB ELF/ZIP alignment are inspected. Host tests and an ash/Vulkan
smoke exercise verify editing, persistence, resize and renderer recreation.
[STATUS](../STATUS.md) records the exact results and unexecuted physical-device
gates. Engineering acceptance does not claim physical Android execution or mobile
performance. Rendering selection beyond this reference remains ADR-0006/M2 work.

## v0.2 lifecycle implementation note

Attached OnePlus13 testing exposed duplicate NativeActivity initialization during
repeat launches. The activity is now singleTask and handles logical Back explicitly.
A narrowly [vendored winit patch](../../vendor/winit/MATTERWEAVE-PATCH.md) handles
Destroy and sequential event-loop recreation, retaining single-live-loop exclusion.
Three same-process Back/relaunch cycles and both landscape orientations passed;
see [device evidence](../evidence/2026-09-07-v0.2.md). Rust/native architecture and
the Vulkan profile remain unchanged. Store signing remains outside this prerelease.
