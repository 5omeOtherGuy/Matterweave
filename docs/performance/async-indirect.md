# Background indirect-light preparation

The render crate owns a bounded CPU lighting controller using the existing
single-bounce `IndirectVolume` producer.

## What problem this solves

Preparing indirect light for a world snapshot is CPU work. Running it on the
owner thread stalls frame submission behind a Vulkan fence. The controller moves
that preparation to one background worker while the owner keeps presenting.

## How it works

### Controller and reuse

One worker prepares a copy-on-write `World` snapshot in 4096-ray/work slices.
One pending snapshot and one completed volume are retained; requests supersede
older work. No Vulkan object crosses into the CPU worker.

The controller reuses the corrected
[`AsyncDetailCollision`](async-detail-collision.md) queue pattern and
standard-library thread/`Mutex`/`Condvar` primitives; no general task framework
or new dependency was needed. The existing ray producer, validation, palette and
radiance arithmetic are reused unchanged.

- Requests clone the source only when queued; duplicate pending, running or
  buffered requests avoid another snapshot.
- Call `request` when the source or light changes. Callers retain an already
  published cache, avoiding re-requesting a consumed unchanged result.
- World replacement must supply a fresh caller epoch.

### Publication keys and cancellation

Publication eligibility is determined by matching source revision, caller
replacement epoch, normalized sun and an opaque reset generation. Reset uses a
new `Arc` identity even after its display counter saturates. A late result cannot
overwrite a newer requested source, and a foreign `poll` cannot consume newer
buffered work. Invalid sun inputs leave valid queued work intact. Shutdown checks
occur between slices. Allocation, snapshot destruction and scheduling also
contribute to latency; no hard wall-clock deadline is claimed.

The renderer owner receives a completed volume through `poll` and performs the
existing fence-safe GPU upload. Source and GPU upload costs remain on the owner
thread.

### Bounds and scope

The bound includes a pending and running source snapshot plus buffered/running
radiance storage. Chunk payloads share through `World` COW; map metadata still
copies. This does not make GI subvoxel-aware or add reflections, sky, emissive or
additional bounces.

## What was verified

### Host tests

At `2267ad5`, 25 focused tests pass. They compare nonzero colored bounce against
the synchronous reference with different work slicing, check light/edit/replacement
invalidation and exercise deterministic reversal/reset/shutdown interleavings.

Two initial gaps were found and closed: the API-only startup test was too weak to
prove worker execution, so the lead extended it to require real request
completion — the partial worker stubs failed and the recovered full
implementation passed. A further shutdown queue test failed before the explicit
refusal guard and passes after it. The original controller tests consumed
buffered results before checking them and asserted race-sensitive queue counts;
both were repaired. Tests also cover open-room radiance and deterministic queue
transitions. Worker completion alone was not treated as acceptance. Strict
Clippy initially caught a test-only field reassignment; initializing the
saturated counter directly fixed it, after which the focused test and strict
workspace Clippy passed. The combined 351-test workspace suite passed.

### Host native Vulkan check

The native `--async-engine-check` passes with llvmpipe Vulkan 1.4.318 validation
and no errors. It rendered 15 frames while background work was pending, then
published matching radiance through all off/on/sun/roof phases. Measured owner
request durations were 0.007–0.008 ms, uploads 0.100–1.210 ms and request-to-poll
latency 22.537–33.905 ms in this one functional host run. These are distinct wall
intervals; they are not Android performance measurements or an efficiency
comparison. Raw report/log:
`/mnt/bench/matterweave-dev/performance/engine-02/async-native`.

### Android device

The Android one-shot marker is `files/engine-check.txt` containing
`indirect-async`. The report is `files/async-engine-check-report.txt`. While
waiting, the fixture continues presenting direct-only frames, then holds
completed phases for 120 presentations. HOME/resume retains source/cache state
and recreates GPU resources.

The v0.5 ARM64 APK builds and passes native page-size, alignment and signing
checks. Its first physical attempt encountered the secure Android lock screen and
suspended before phase 0. The attempt timed out and is not a functional pass. The
subsequent unlocked run passed all six phases, HOME/resume and renderer
recreation on OnePlus 13 CPH2653, Android 16, Adreno 830, Vulkan 1.3.284, driver
2150760522. `capture.json` records `terminal_pass=true`. Request handling was
0.034–0.064 ms, request-to-poll 22.903–48.667 ms, and ordinary publication
uploads 3.429–4.931 ms. Nine frames presented during preparation; recreation
re-uploaded cached radiance in 0.185 ms. These are short functional observations,
not sustained frame-time or thermal evidence. Phone Vulkan validation was
disabled.

APK source: `9854723876e075c908cf1524d2b27b6bf60fcb61`; SHA-256:
`dc1d7108d01de4f861a95cadb54085b58248685c466ae8dfde5a6ccc08b807ad`.
Raw evidence: `/mnt/bench/matterweave-dev/performance/engine-02/phone-async-indirect`.
The separate `locked-attempt-01` is retained as failed evidence.

### Standalone CPU example

The standalone `async_indirect` example at `70a231a` also passed as an ARM64
Android executable. It compares every cell/face of open, closed and reopened
voxel enclosures with the synchronous producer. Open/reopened samples were
`[0.25312495, 0.014062502, 0.005625]`; closed was zero. This supplies CPU
correctness evidence only. Its manifest and report are under
`/mnt/bench/matterweave-dev/performance/engine-02/phone-async-cpu`. The
[retained manifest](../evidence/2026-09-08-async-lighting.json) records exact
source and evidence checksums. The [v0.5 evidence archive](https://github.com/5omeOtherGuy/Matterweave/releases/download/v0.5.0/matterweave-v0.5-lighting-evidence.tar.gz)
retains these records and a fresh six-phase phone pass at `577dff8`. Its SHA-256
is `07301d6e20d55310eedab48c797fde930464e180021d9c82f28bb38284523247`.

### Instrumented controller coverage

The 25 focused tests also passed with only the render test target instrumented:
`cargo rustc --locked -p matterweave-render --lib --profile test -- -C instrument-coverage`.
Run the emitted test binary filtered to `async_indirect`, with `LLVM_PROFILE_FILE`
set to a dedicated path containing `%m-%p`, merge with matching `llvm-profdata`,
then report `async_indirect.rs` with `llvm-cov`. The production controller maps
260 lines, 240 executed (**92.31%**), and 27 functions, all executed. No counter
mismatch warnings occurred. This excludes the test file and does not measure
Android execution, shader coverage or branch coverage. Raw commands/profile/report
are under `/mnt/bench/matterweave-dev/performance/engine-02/async-coverage`.

## Limits and open work

- CPU preparation progresses without holding a Vulkan frame fence. Visible
  direct-only waiting frames demonstrate scheduling behavior; temporal quality,
  production cadence, full-scene cost and sustained mobile efficiency remain
  open.
- The existing synchronous check remains available as a reference.
