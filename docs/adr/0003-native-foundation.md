# ADR-0003: Native language, platform and graphics foundation

- Date: 2026-09-07
- Status: Proposed
- Basis: Prior technical recommendation and initial engineering hypothesis, not an owner language choice.
- Requirements: R01, R03, R10, R13

## Context

The engine needs low-level Android integration, graphics control, native physics/library interoperability and reproducible builds. Android identifies Vulkan as its primary low-level graphics API.

## Decision

Propose C++20 for the engine, CMake for native builds, Gradle for Android packaging and a thin Kotlin/Java shell only where platform integration needs it. Start with ARM64 and Vulkan. Evaluate GameActivity, frame pacing and other Android game libraries; adopt only what the first slice uses.

M0 must choose and pin a compatible toolchain, minimum API and Vulkan feature profile. The proposal does not fix a Vulkan version, require hardware RT, mandate unnecessary language features or select a shader compiler. Keep a host-testable core with platform adapters.

## Alternatives

Rust with Vulkan or wgpu is credible, particularly for voxel libraries, but changes dependency/FFI and renderer-control tradeoffs. A full existing engine can reduce tooling work. OpenGL ES could be a later compatibility path if justified; it is not the proposed primary API.

## Consequences

Direct Vulkan introduces synchronization, resource lifetime and device-compatibility work. Native libraries and APK packaging need page-size validation. A thin platform shell avoids implementing Android lifecycle mechanics throughout the engine.

## Validation

Accept/revise in M0 after a reproducible APK build, documented lifecycle/input checks, toolchain compatibility and capability inventory. Physical-device absence limits runtime evidence; it does not prevent choosing a buildable foundation provisionally.

## References

[Android Vulkan overview](https://developer.android.com/games/develop/vulkan/overview), [AGDK](https://developer.android.com/games/agdk/overview), [development guide](../DEVELOPMENT.md).
