# Development guide

## Available now

This setup contains documentation and its validator. It does not contain a Cargo engine workspace, Gradle application, Android manifest, native source or buildable APK. Rust preference is accepted; exact implementation packages/toolchains remain proposals.

```sh
git clone https://github.com/5omeOtherGuy/Matterweave.git
cd Matterweave
python3 tools/check_docs.py
```

Python 3.10+ is sufficient for the documentation check. It uses only the standard library and checks local links and ADR/requirement consistency. External URLs, Android compatibility and architectural feasibility are not validated by this command.

## M0 environment setup requirements

Inspect the actual development environment before selecting tools. Resolve a compatible stable Rust/Cargo and JDK/Gradle/Android Gradle Plugin/SDK/NDK set from official documentation; add CMake only for components that require it. Pin exact versions, the Rust edition/MSRV, rust-toolchain.toml, workspace Cargo.lock and the Gradle wrapper with checksum verification. Choose and record the initial minimum Android API, ARM64 ABI and Vulkan feature profile. Accepted ADR-0014 governs selection; ADR-0015 proposes the implementation foundation.

Document installation steps for at least one reproducible host environment, required environment variables and a clean-checkout build. Commit a dependency manifest/lock mechanism with exact revisions, source URLs, licenses, local patches and rationale when dependencies are first adopted. No dependency has been pinned in this setup, and reference repositories are not automatically dependencies.

Assess suitable Rust components under [component selection](COMPONENT_SELECTION.md). Prototype missing interfaces or significant improvements only against a concrete gap. Do not replace the existing adequate Python documentation validator solely for language uniformity. Kotlin/platform glue and GPU shaders remain legitimate other-language components where appropriate.

Select shader source language/compiler and record how shaders are built, reflected and packaged. Do not rely on a developer's globally installed unversioned compiler. Inspect [Android native page-size guidance](https://developer.android.com/guide/practices/page-sizes) for every native library and the resulting APK; avoid assumptions about a fixed 4 KB page size.

## Commands the implementation must add

Add and verify exact commands for native/host configure, build and tests; Android debug/profile build; APK location; device discovery; installation and launch; diagnostics capture; and the deterministic benchmark runner. Include expected output and common environment failures. Do not document guessed module names or present commands for nonexistent targets as working instructions.

Prefer debug signing for development. Do not commit keystores, access tokens, local SDK paths or release credentials. Store-specific publication and signing are not part of this setup.

## Android functional checks

Test launch, simultaneous touches, input cancellation, background/foreground, surface loss/recreation, safe shutdown, display changes and save/reload. Choose a documented orientation policy rather than allowing accidental behavior. Handle unsupported Vulkan/device features with a clear diagnostic or supported fallback. Avoid claiming desktop input proves touch usability.

When real hardware is unavailable, produce the APK, use available host/emulator checks and leave precise physical-device steps. Maintain a distinction between build success, functional success and measured device performance.

## Build and CI progression

The initial documentation workflow checks repository integrity. M0 should add Rust formatting, lint and host tests plus a clean Android APK build with artifact retention. Record actual commands once targets exist. Record toolchain versions and use reproducible dependency retrieval. Document unsafe/FFI invariants; add Vulkan validation/debug builds and applicable sanitizer or Miri checks for specific correctness risks, without treating Miri as GPU/foreign-library validation. Keep release performance captures separate from instrumented correctness runs.

Cache downloads/build products using appropriate version keys. Do not make a cached build the only evidence that a fresh checkout works. Keep CI permissions minimal and avoid exposing credentials in logs.

## Contribution and evidence

Follow [CONTRIBUTING.md](../CONTRIBUTING.md), [AGENTS.md](../AGENTS.md) and the [benchmark protocol](BENCHMARKS.md). Update [STATUS.md](STATUS.md) and affected ADRs with each meaningful implementation handoff. Prefer clear modules and small complete changes over placeholder systems.
