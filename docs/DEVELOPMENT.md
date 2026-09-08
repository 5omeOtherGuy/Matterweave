# Development guide

The workspace contains `matterweave-core`, `matterweave-physics` (Rapier), `matterweave-render` (ash/Vulkan), and
`matterweave-explorer`. Android NativeActivity loads the explorer shared library;
a desktop executable supplies supporting integration tests. See [MVP scope](MVP.md)
and [status](STATUS.md) for implemented behavior and actual evidence.

## Linux build setup

Use Linux x86_64, Python 3.11+, Rustup, JDK 21, unzip, and network access to the
pinned upstream sources. The docs validator alone still supports Python 3.10+.
Dependencies and build tools are listed in [the component record](DEPENDENCIES.md).

```sh
git clone https://github.com/5omeOtherGuy/Matterweave.git
cd Matterweave
rustup toolchain install 1.96.0 --profile minimal --component rustfmt,clippy
export ANDROID_HOME="$HOME/Android/Sdk"
python3 tools/setup_android.py --sdk "$ANDROID_HOME"
```

The setup helper validates the command-line tools archive SHA-256 and invokes
SDK manager for API 35, build-tools 35.0.0 and NDK 28.2.13676358. Review the SDK
licenses when prompted; `--accept-licenses` is available for authorized CI setup.
It adds the `aarch64-linux-android` Rust target. It does not change global Rust,
Gradle or shell configuration. Existing SDK installations are supported.

Allow several GB for the SDK, native build outputs and dependency caches. You may
set `CARGO_TARGET_DIR` and `GRADLE_USER_HOME` to directories on a larger volume;
these are optional location overrides, never committed machine-specific paths.
Use a separate target directory for worktrees with different source revisions:
sharing one caused stale Android local-crate metadata during the performance
campaign (missing a newly exported type despite correct source). Dependency
caches can be shared; compiled local-crate targets must not be assumed portable
between worktrees.
The Gradle wrapper validates its distribution and Maven artifact checksums.
Android ABI/minimum profile is ARM64, API 28 and Vulkan 1.1. This is a development
profile, not a store compatibility or device-performance guarantee.

## Host checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 tools/check_docs.py
python3 tools/dependency_report.py
```

The core tests cover independent reference edits, negative/chunk boundaries,
ray queries, mesh winding/occlusion, bounded snapshot validation, save failures,
round trips and revision exhaustion. Explorer tests cover simultaneous touch
roles, cancellation, camera bounds, actual sample editing, save retry and corrupt
save recovery. Production core coverage reproduction is in its
[README](../crates/matterweave-core/README.md); coverage is not GPU validation.

## Build and inspect the Android APK

```sh
android/gradlew -p android :app:assembleDebug --no-daemon
python3 tools/verify_apk.py android/app/build/outputs/apk/debug/app-debug.apk
"$ANDROID_HOME/build-tools/35.0.0/zipalign" -c -P 16 -v 4 android/app/build/outputs/apk/debug/app-debug.apk
"$ANDROID_HOME/build-tools/35.0.0/apksigner" verify --verbose android/app/build/outputs/apk/debug/app-debug.apk
"$ANDROID_HOME/build-tools/35.0.0/aapt" dump badging android/app/build/outputs/apk/debug/app-debug.apk
sha256sum android/app/build/outputs/apk/debug/app-debug.apk
```

Gradle invokes `tools/build_native.py`, which uses `cargo build --locked`, the
pinned NDK ARM64/API-28 compiler, and an explicit 16 KiB maximum-page-size linker
option. The library is built as `libmatterweave_explorer.so`. AGP packages native
libraries uncompressed and aligned; the inspector checks every packaged `.so`
for ARM64 ELF LOAD and ZIP-entry alignment. Debug signing uses the local standard
Android debug key, never committed. Successful packaging does not prove device
execution, Vulkan driver correctness or lifecycle behavior.

The supported artifact is the development APK. The owner requested delivery via
[GitHub prereleases](https://github.com/5omeOtherGuy/Matterweave/releases), with
an APK checksum and its build/test limitations. Production signing and store
publication have not been configured. The Android CI job runs the same build and
retains the APK and SHA-256 as a workflow artifact. CI execution status must be
checked separately from the locally executed commands in STATUS.

## Install, launch and collect Android evidence

```sh
export PATH="$ANDROID_HOME/platform-tools:$PATH"
adb devices -l
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n dev.matterweave.explorer/android.app.NativeActivity
adb logcat -d -s Matterweave '*:S'
```

For multiple devices, add `-s SERIAL` to every adb command. USB debugging and host
authorization must already be available. No physical device is implied by the
presence of `adb`. Save files live at `internal_data_path()/world.json`; no network,
external-storage permission, cloud account or service is required.

Touch and keyboard controls for walking, flight, terrain edits and physical object
interaction are listed in [v0.2](V0.2.md#touch-controls). Edits save immediately;
terrain, object state, camera and control preferences share one atomic snapshot.
Existing v0.1 saves migrate automatically. Back exits the activity; HOME inside the
app respawns the player. These are distinct from the phone's Home navigation action.

Physical-device checklist (record each result, do not infer from host tests):

1. Launch in landscape; verify correct orientation, readable controls, visible
   terrain and queried GPU/driver diagnostics.
2. Move/look with two fingers and edit with a third. Release, cancel, lose focus
   and resume; confirm no stuck movement/look. Try both layout sizes/sides.
3. Place/remove terrain across a chunk boundary. Relaunch after an edit; verify
   persistence and material/query agreement. Aim beyond range for a harmless miss.
4. Home/resume repeatedly, lock/unlock, rotate between landscape orientations,
   and test display/surface recreation. The current path requires identity
   surface transform support and lets the Android compositor rotate.
5. Inspect logcat for initialization, allocation or Vulkan errors. Confirm the
   application does not render while suspended. Test low-memory behavior where
   feasible; preserve any failure logs.
6. Record model, OS/API, driver, build commit, APK checksum, seed, resolution and
   conditions in a new evidence report. Follow [BENCHMARKS](BENCHMARKS.md) before
   making performance claims; host/emulator timings do not select a mobile renderer.

## Supporting native host run

A Vulkan loader and compatible driver are required. Host software Vulkan is
useful for correctness only. Debug builds enable the validation layer if installed;
`MATTERWEAVE_VALIDATION=1` also requests it in release, while `0` disables it.
Missing layers are reported in the capabilities line.

```sh
cargo run --locked -p matterweave-explorer -- --save /tmp/matterweave-world.json
```

Desktop controls are listed in [v0.2](V0.2.md#touch-controls). The desktop
launcher is supporting evidence; Android remains the product platform.

For a headless integration run on Linux, install Xvfb, xauth, Mesa Vulkan drivers
and Vulkan validation layers through your package manager. Use a new temporary
save path because the exercise deliberately verifies fresh save/load behavior:

```sh
cargo build --locked -p matterweave-explorer --bin matterweave-explorer
smoke_dir=$(mktemp -d)
timeout 90s xvfb-run -a cargo run --locked -p matterweave-explorer -- --smoke-exercise --smoke-frames 30 --save "$smoke_dir/world.json"
```

`--smoke-exercise` requires an explicit nonexistent save path. It tests actual
sample place/remove plus grab/throw/fracture and atomic save/load actions, observes a resize event, recreates the host
window/renderer, and exits after the requested presented-frame count. Failure
exits nonzero. This exercises shared code, not Android lifecycle callbacks on a
phone. Inspect validation output as well as exit status.

## Opt-in native detail/flora viewer

See [gallery mode](performance/p03/native-gallery.md) for marker grammar, strict
user-save isolation and remaining device/quality limits. `flora source` loads the
reviewed84-plant fixture once; it does not select an accepted full-map showcase.

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/target
cargo build --locked -p matterweave-explorer --bin matterweave-explorer
python3 tools/performance/check_gallery_capture.py "$CARGO_TARGET_DIR/debug/matterweave-explorer" /mnt/bench/matterweave-dev/performance/gallery-tile-check
python3 tools/performance/check_gallery_capture.py "$CARGO_TARGET_DIR/debug/matterweave-explorer" /mnt/bench/matterweave-dev/performance/gallery-flora-check --preset flora
```

Choose fresh artifact directories on repeats. These actual Vulkan/Xvfb checks
retain logs,20 schema-valid capture rows and a deliberately invalid user-world
sentinel. A compatibility-mesh upload must not be mislabeled as dynamic-body
work. CI runs both checks after building the native binary; inspect their logs
for validation errors too.

For device recipes, stop the app **after** installing/reinstalling an APK before
restoring its save. Use `adb shell -T run-as ...` with the shell exit-status protocol
for synchronous writes; verify original bytes afterwards. Stream app-scoped
logcat before launch for startup diagnostics; a late dump can lose early lines.
Do not weaken Android security properties to obtain simpleperf access.

## Diagnostics and current limits

FRAME is a smoothed interval between redraws; MAIN is wall time inside draw,
including Vulkan waits; MESH is the most recent changed-chunk extraction/upload wall time.
These are not GPU timestamps or CPU execution samples. DATA counts voxel payload,
not process memory. GPU heap sizes are queried capacities, not free memory.

Terrain generation and dirty chunk meshing now use bounded background preparation.
The normal app still publishes collision and uploads GPU resources on its owner
thread. Engine APIs additionally support background detail-collision preparation,
shared chunk snapshots, automatic LOD and an opt-in diffuse-light reference. Greedy
chunk meshes, conservative frustum culling and filtered cached shadows are implemented.
See STATUS for native/Android checks and remaining scheduling/quality limits. Physics runs
at 60 Hz with bounded catch-up and interpolated object rendering. The resident
window, stored-override limit and save size are explicit in the core README.
Corrupt world/session snapshots remain intact; numbered recovery snapshots are
loaded on subsequent startup. Errors are shown in the HUD and logged.

The published development APK uses optimization level 2 and the existing local
debug key. CI uses its own debug key; installing a CI artifact over the GitHub
release can fail signature validation. Use GitHub release APKs for updates.

The Android lifecycle correction is a narrowly vendored winit 0.30.12 patch; see
[vendor provenance](../vendor/winit/MATTERWEAVE-PATCH.md). It handles activity destruction
and sequential event-loop recreation. The singleTask activity manifest routes normal
repeat launches to the existing NativeActivity, avoiding duplicate NDK contexts.

The extra renderer cache smoke can be run with:

```sh
timeout 60s xvfb-run -a cargo run --locked -p matterweave-render --example cache_smoke
```

It covers submitted-buffer replacement, stale/invalid uploads, eviction, culling,
dynamic buffer reuse/growth and teardown under Vulkan validation.

Changing dependencies requires regenerating Cargo.lock and provenance intentionally.
For a deliberate Gradle dependency update, regenerate verification metadata from
trusted upstreams with `--write-verification-metadata sha256`, review it, then rerun
a normal verification-enforced build. Do not disable verification to bypass failures.

## Opt-in frame capture (performance campaign development)

Place an integer count (1..240000) in `profile-frames.txt` beside the world save
before launching the app. On Android this is the app-private `files` directory,
accessible through `adb shell run-as dev.matterweave.explorer`. A valid request is
consumed after a new timestamped `frame-profile-*.csv` opens successfully. Captures
never overwrite an existing file, stop at the requested count and flush on suspend
or exit. Invalid requests remain for correction. Normal runs create no frame log.

The current development branch writes [typed schema v2](performance/measurement-v2.md)
to `frame-profile-v2-*.csv`; the shipped v0.3 APK writes the historical v1 format.
Do not parse either format as the other. V2 records exact draw/submission/completion
IDs and renderer epochs, main-thread CPU busy time where supported, stage walls,
separate mesh-sync/render fence waits and actual work/save-attempt counters. Retries
may occur before or after submission; successful present API calls are not scanouts.
Missing measurements remain empty, never fabricated zeroes. GPU timestamp spans can
include queue stalls and are not pure shader execution or compositor timestamps.

Copy captures under `/mnt/bench` and preserve build/scene/conditions before making
performance comparisons. New instrumentation overhead and same-build repeatability
must be qualified on the phone; host correctness and an APK build do not establish
that qualification.

Performance tool and deterministic host-fixture checks:

```bash
python3 -m unittest discover -s tools/performance -p 'test_*.py'
cargo test --locked -p matterweave-core --test performance_replay
```

`tools/performance/check_handoff.py BOARD_JSON SUBMISSION_JSON ARTIFACT` validates
a frozen patch admission against the sole lead-written board; see
[the handoff contract](performance/README.md). It is not a process supervisor.
`tools/performance/validate_conditions.py HEALTH_JSONL` checks two-minute **idle
readiness**, not app performance. Rows use actual `elapsed_s` and `data.battery` /
`data.thermalservice` raw dumps; current HAL skin readings, actual power-state
fields and non-overridden thermal status are required. Never substitute synthetic
times into real readiness evidence. The replay fixture currently proves host world
regressions only; app/phone input-replay integration remains outstanding.

Coordination startup/recovery checks:

```bash
python3 -m unittest discover -s tools/coordination -p 'test_*.py' -v
```

See the [board operations guide](../tools/coordination/README.md) and
[v0.3 protocol](V0.3.md) for bounded worker reads and ownership rules.

## Full wetland integration checks

The complete map remains a separate native app mode, entered through the ordinary
chooser or the host `--showcase` flag. Run the expensive real-runtime regression
explicitly after map/collision/app changes:

```sh
cargo test --locked -p matterweave-explorer --lib full_wetland_load_edit_collision_and_reload -- --ignored --nocapture
cargo test --locked -p matterweave-physics --test showcase_traversal -- --ignored --nocapture --test-threads=1
cargo build --locked -p matterweave-explorer --bin matterweave-explorer
MATTERWEAVE_VALIDATION=1 python3 tools/performance/check_wetland_capture.py target/debug/matterweave-explorer /mnt/bench/matterweave-wetland-check
```

Use a fresh artifact directory. On a host with hardware ICDs that cannot present
to Xvfb, explicitly select the installed lavapipe ICD with `VK_ICD_FILENAMES`;
software-renderer timings are correctness evidence only. The capture verifier
requires25 presented frames,20 schema-valid records with actual simulation, six
arch bodies, an isolated wetland save and no Vulkan validation errors. It retains
failure artifacts and terminates its owned process group on timeout.


Native route replay cancellation can be checked through the actual app:

```sh
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json MATTERWEAVE_VALIDATION=1 python3 tools/performance/check_wetland_capture.py target/debug/matterweave-explorer /mnt/bench/matterweave-replay-check --check-replay-cancel
```

The request advances the normal simulation, then the explicit25-frame smoke exit
must record terminal CANCEL. This is not a full route pass. Phone requests and
isolated fixture requirements are in [route replay](performance/showcase/route-replay.md).
The [paired collector](performance/p02-phone-runner.md) records fresh readiness,
serialized installs and actual compositor/app evidence; its `--help` is safe to
run without a phone. Only one collector/device owner may run at a time.


Offline paired summaries use the [analysis tool](performance/p02-analysis.md):

```sh
python3 tools/performance/analyze_wetland_pairs.py --input /absolute/completed-or-partial-batch --out /absolute/fresh-analysis
python3 -m unittest discover -s tools/performance -p 'test_*.py'
```

Only completed trial directories are analyzed. Keep separately collected batches
separate; compare their within-pair differences after controls and image review.


Engine shadow-cache verification (host Vulkan, no mobile performance inference):

```sh
MATTERWEAVE_VALIDATION=1 timeout 90s xvfb-run -a cargo run --locked -p matterweave-render --example shadow_cache_smoke
```

The30-frame check covers initial shadows-off initialization, stationary reuse,
intensity/view changes, moving geometry with unchanged revisions, chunk replacement
and eviction, static transforms, failed update retention, sun/camera/resolution
changes, hidden frames and renderer recreation. It verifies GPU timing availability
for actual depth passes versus reuse. CI runs it alongside the existing Vulkan checks.

## Engine indirect-light check

The opt-in test runs the actual Vulkan diffuse-light cache through off/on, moving
sun, closed/open enclosure, stale-edit rejection and light invalidation phases.
It uses a small disposable unit-voxel fixture and does not load a sample world.

```sh
cargo run -p matterweave-explorer -- --engine-check --save /tmp/unused.json
cat /tmp/engine-check-report.txt
```

On a debuggable Android APK, write `indirect` into the application files directory
as `engine-check.txt` with `adb shell run-as`, then launch the activity. The request
is consumed once before the world chooser. Read `engine-check-report.txt` from
that directory for phase observations and the terminal `PASS indirect` marker.
Android phases retain120 presented frames each for external screenshots; desktop
uses6. HOME releases the renderer and resume recreates it before republishing the
cache. A successful build alone does not count as running this check.

## Engine automatic detail check

Run `cargo run --locked -p matterweave-explorer -- --detail-check --save /tmp/unused.json`
and require `PASS detail` in `/tmp/detail-check-report.txt`. On Android the same
one-shot mechanism above accepts `detail` and writes `detail-check-report.txt`.
The9 phases exercise retained geometry with automatic perspective/orthographic
LOD, an edit, zero extent and recreation. See [evidence and limits](performance/detail-native-check.md).

## Background indirect-light check

Run `cargo run --locked -p matterweave-explorer -- --async-engine-check --save /tmp/unused.json`
and require `PASS async indirect` in `/tmp/async-engine-check-report.txt`. Android
accepts the one-shot marker value `indirect-async`. The fixture presents direct-only
frames during CPU preparation, then uploads completed matching radiance. Reports
separate owner request/upload time from request-to-poll latency. See
[controller evidence and limits](performance/async-indirect.md).

### Standalone background-light correctness check

`cargo run --locked -p matterweave-render --example async_indirect` compares all
cells/faces of an open, closed and reopened enclosure against synchronous lighting.
CI executes it. For Android, build the same example with the pinned NDK compiler:

```sh
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_HOME/ndk/28.2.13676358/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android28-clang"
cargo build --locked -p matterweave-render --example async_indirect --target aarch64-linux-android
adb push "$CARGO_TARGET_DIR/aarch64-linux-android/debug/examples/async_indirect" /data/local/tmp/matterweave-async-indirect
adb shell chmod 755 /data/local/tmp/matterweave-async-indirect
adb shell /data/local/tmp/matterweave-async-indirect
```

Set `CARGO_TARGET_DIR` to a dedicated build directory before these commands.
This headless test proves CPU-controller correctness only; the APK adapter check
above separately exercises presentation, upload and lifecycle behavior.

### Sustained native streaming correctness gate

```sh
cargo run --locked -p matterweave-core --example stream_stress -- 2 /tmp/matterweave-stream-new.json
```

The second argument must be an unused disposable save path. A successful run
removes only that test-created file; failed runs retain it for diagnosis. Duration
is 1–1800 seconds, with at least six route changes. CI executes the short gate.
Cross-compile `--example stream_stress` using the same Android linker as the
standalone lighting check, push it under `/data/local/tmp`, and run it with
`600 /data/local/tmp/matterweave-stream-new.json` for a ten-minute gate.
Record exact source/binary checksum, device and build configuration alongside
stdout and health observations. See [streaming stress](performance/stream-stress.md).

Full-image ray/raster/shared-depth hybrid correctness: `cargo run --locked -p matterweave-render --example renderer_comparison`. See [comparison protocol](performance/renderer-comparison.md) for tolerances, artifacts and Android evidence.
