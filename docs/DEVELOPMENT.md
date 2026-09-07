# Development guide

## Available now

This setup contains documentation and its validator. It does not contain Gradle, CMake engine targets, an Android manifest, native source or a buildable APK.

```sh
git clone https://github.com/5omeOtherGuy/Matterweave.git
cd Matterweave
python3 tools/check_docs.py
```

Python 3.10+ is sufficient for the documentation check. It uses only the standard library and checks local links and ADR/requirement consistency. External URLs, Android compatibility and architectural feasibility are not validated by this command.

## M0 environment setup requirements

Inspect the actual development environment before selecting tools. Resolve a compatible JDK/Gradle/Android Gradle Plugin/SDK/NDK/CMake set from official documentation, pin exact versions, and commit the Gradle wrapper plus checksum verification. Choose and record the initial minimum Android API, ARM64 ABI and Vulkan feature profile. The native foundation in ADR-0003 is a proposal until verified.

Document installation steps for at least one reproducible host environment, required environment variables and a clean-checkout build. Commit a dependency manifest/lock mechanism with exact revisions, source URLs, licenses, local patches and rationale when dependencies are first adopted. No dependency has been pinned in this setup, and reference repositories are not automatically dependencies.

Select shader source language/compiler and record how shaders are built, reflected and packaged. Do not rely on a developer's globally installed unversioned compiler. Inspect [Android native page-size guidance](https://developer.android.com/guide/practices/page-sizes) for every native library and the resulting APK; avoid assumptions about a fixed 4 KB page size.

## Commands the implementation must add

Add and verify exact commands for native/host configure, build and tests; Android debug/profile build; APK location; device discovery; installation and launch; diagnostics capture; and the deterministic benchmark runner. Include expected output and common environment failures. Do not document guessed module names or present commands for nonexistent targets as working instructions.

Prefer debug signing for development. Do not commit keystores, access tokens, local SDK paths or release credentials. Store-specific publication and signing are not part of this setup.

## Android functional checks

Test launch, simultaneous touches, input cancellation, background/foreground, surface loss/recreation, safe shutdown, display changes and save/reload. Choose a documented orientation policy rather than allowing accidental behavior. Handle unsupported Vulkan/device features with a clear diagnostic or supported fallback. Avoid claiming desktop input proves touch usability.

When real hardware is unavailable, produce the APK, use available host/emulator checks and leave precise physical-device steps. Maintain a distinction between build success, functional success and measured device performance.

## Build and CI progression

The initial documentation workflow checks repository integrity. M0 should add host tests and a clean Android APK build with artifact retention. Record toolchain versions and use reproducible dependency retrieval. Add Vulkan validation/debug builds and targeted sanitizers where supported; keep release performance captures separate from instrumented correctness runs.

Cache downloads/build products using appropriate version keys. Do not make a cached build the only evidence that a fresh checkout works. Keep CI permissions minimal and avoid exposing credentials in logs.

## Contribution and evidence

Follow [CONTRIBUTING.md](../CONTRIBUTING.md), [AGENTS.md](../AGENTS.md) and the [benchmark protocol](BENCHMARKS.md). Update [STATUS.md](STATUS.md) and affected ADRs with each meaningful implementation handoff. Prefer clear modules and small complete changes over placeholder systems.
