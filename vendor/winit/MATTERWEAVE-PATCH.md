# Matterweave Android lifecycle patch

Upstream package: winit 0.30.12, Apache-2.0; its original `LICENSE`, package
manifest, source, examples and tests are retained. This directory is the unpacked
crates.io package, excluding Cargo's local `.cargo-ok` marker, plus this note,
`MATTERWEAVE.patch`, and the two source-file changes recorded by that patch.

- Package: <https://crates.io/api/v1/crates/winit/0.30.12/download>
- Package SHA-256: `c66d4b9ed69c4009f6321f762d6e61ad8a2389cd431b97cb1e146812e9e6c732`
- Upstream revision from `.cargo_vcs_info.json`:
  `f6893a4390dfe6118ce4b33458d458fd3efd3025`
- Source: <https://github.com/rust-windowing/winit/tree/v0.30.12>
- Audited on 2026-09-07. The latest compatible published version was 0.30.13;
  its Android `MainEvent::Destroy` handler still ignored destruction, and its
  process creation flag still allowed recreation only on web. An upgrade therefore
  did not resolve these two defects. No dependencies were upgraded for this patch.

## Changes and lifetime reasoning

1. Android `MainEvent::Destroy` stops dispatch, marks the event loop exiting and
   returns to the existing `pump_events` exit path. That path emits `LoopExiting`
   exactly as for an application-requested exit. Matterweave's `exiting` callback
   saves its session and releases renderer/window resources. `run_app` can then
   return and `android_main` can finish. This matters because android-activity
   0.6.0's NativeActivity `onDestroy` waits for its Rust main thread to stop.
2. The Android platform loop owns a zero-sized creation guard as its last field.
   Rust drops preceding fields first; only then does the guard clear winit's
   global creation flag. The platform `run` function consumes and owns this
   structure throughout the loop, so moving the public EventLoop into `run_app`
   does not release the guard prematurely. Dropping an unrun loop also releases
   it. The global acquisition remains atomic and rejects a second live loop;
   acquisition uses AcqRel and release uses Release. Other platforms retain their
   previous recreation policy.

NativeActivity's Java main thread waits for the old Rust main thread to finish
before completing destruction. android-activity releases `ndk_context` before
signalling that completion. The next Activity may then create a new event loop.
This patch does not permit two simultaneous NativeActivity instances: the app's
`singleTask` manifest setting separately routes ordinary repeated launches to the
existing root activity.

## Reproduction and verification

Root Cargo configuration uses a `[patch.crates-io]` path override and excludes
this source directory from workspace members. Cargo.lock and this package checksum
pin provenance; `MATTERWEAVE.patch` is a normal unified diff against the package.
To reproduce, download and verify the package archive, unpack it, then run
`patch -p1 < MATTERWEAVE.patch` from the unpacked winit directory.

Host renderer smoke checks continue to cover the unmodified host platform. An
ARM64 Android build checks the changed platform code. Actual activity destruction
and recreation must also be exercised on a device; a host build cannot validate
Android callback ordering. Concrete build/device results belong in the root
`docs/STATUS.md` and its evidence references.

When updating winit, first check whether upstream implements both fixes; remove
this override when equivalent lifecycle behavior is verified.
