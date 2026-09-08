# Background indirect-light preparation

The render crate now owns a bounded CPU lighting controller using the existing
single-bounce `IndirectVolume` producer. One worker prepares a COW World snapshot
in4096-ray/work slices. One pending snapshot and one completed volume are retained;
requests supersede older work. Matching source revision, caller replacement epoch,
normalized sun and an opaque reset generation determine publication eligibility.
The renderer owner receives a completed volume through `poll` and performs the
existing fence-safe GPU upload. No Vulkan object crosses into the CPU worker.

## Reuse and ownership

Reuse the corrected `AsyncDetailCollision` queue pattern and standard-library
thread/Mutex/Condvar primitives. No general task framework or new dependency was
needed. The existing ray producer, validation, palette and radiance arithmetic
are reused unchanged. Requests clone source only when queued; duplicate pending,
running or buffered requests avoid another snapshot. Call request when the source
or light changes; callers retain an already published cache, avoiding re-requesting
a consumed unchanged result. World replacement must supply a fresh caller epoch.

Reset uses a new Arc identity even after its display counter saturates. A late
result cannot overwrite a newer requested source; a foreign poll cannot consume
newer buffered work. Invalid sun inputs leave valid queued work intact. Shutdown
checks occur between slices. Allocation, snapshot destruction and scheduling also
contribute to latency; no hard wall-clock deadline is claimed.

The bound includes a pending and running source snapshot plus buffered/running
radiance storage. Chunk payloads share through World COW; map metadata still copies.
Source and GPU upload costs remain on the owner thread. This does not make GI
subvoxel-aware or add reflections, sky, emissive or additional bounces.

## Verification

At `2267ad5`,25 focused tests pass. They compare nonzero colored bounce against the
synchronous reference with different work slicing, check light/edit/replacement
invalidation and exercise deterministic reversal/reset/shutdown interleavings.
The initial API-only startup test was too weak to prove worker execution. Lead
extended it to require real request completion: the partial worker stubs failed,
then the recovered full implementation passed. A further shutdown queue test
failed before the explicit refusal guard and passes after it.

The native `--async-engine-check` passes with llvmpipe Vulkan1.4.318 validation
and no errors. It rendered15 frames while background work was pending, then
published matching radiance through all off/on/sun/roof phases. Measured owner
request durations were0.007–0.008ms, uploads0.100–1.210ms and request-to-poll latency
22.537–33.905ms in this one functional host run. These are distinct wall intervals;
they are not Android performance measurements or an efficiency comparison.
Raw report/log: `/mnt/bench/matterweave-dev/performance/engine-02/async-native`.

The Android one-shot marker is `files/engine-check.txt` containing `indirect-async`.
The report is `files/async-engine-check-report.txt`. While waiting, the fixture
continues presenting direct-only frames, then holds completed phases for120
presentations. HOME/resume retains source/cache state and recreates GPU resources.
The v0.5 ARM64 APK builds and passes native page-size, alignment and signing
checks. Its first physical attempt encountered the secure Android lock screen and
suspended before phase0. The attempt timed out and is not a functional pass. The subsequent unlocked run passed all six phases, HOME/resume and renderer
recreation on OnePlus 13 CPH2653, Android 16, Adreno 830, Vulkan 1.3.284,
driver 2150760522. `capture.json` records `terminal_pass=true`. Request handling
was 0.034–0.064 ms, request-to-poll 22.903–48.667 ms, and ordinary publication
uploads 3.429–4.931 ms. Nine frames presented during preparation; recreation
re-uploaded cached radiance in 0.185 ms. These are short functional observations,
not sustained frame-time or thermal evidence. Phone Vulkan validation was disabled.

APK source: `9854723876e075c908cf1524d2b27b6bf60fcb61`; SHA-256:
`dc1d7108d01de4f861a95cadb54085b58248685c466ae8dfde5a6ccc08b807ad`.
Raw evidence: `/mnt/bench/matterweave-dev/performance/engine-02/phone-async-indirect`.
The separate `locked-attempt-01` is retained as failed evidence.

The standalone `async_indirect` example at `70a231a` also passed as an ARM64
Android executable. It compares every cell/face of open, closed and reopened
voxel enclosures with the synchronous producer. Open/reopened samples were
`[0.25312495, 0.014062502, 0.005625]`; closed was zero. This supplies CPU
correctness evidence only. Its manifest and report are under
`/mnt/bench/matterweave-dev/performance/engine-02/phone-async-cpu`.
The [retained manifest](../evidence/2026-09-08-async-lighting.json) records exact
source and evidence checksums. Durable archive upload remains a delivery task.


## Instrumented controller coverage

The25 focused tests also passed with only the render test target instrumented:
`cargo rustc --locked -p matterweave-render --lib --profile test -- -C instrument-coverage`.
Run the emitted test binary filtered to `async_indirect`, with `LLVM_PROFILE_FILE`
set to a dedicated path containing `%m-%p`, merge with matching `llvm-profdata`,
then report `async_indirect.rs` with `llvm-cov`. The production controller maps
260 lines,240 executed (**92.31%**), and27 functions,all executed. No counter
mismatch warnings occurred. This excludes the test file and does not measure
Android execution, shader coverage or branch coverage. Raw commands/profile/report
are under `/mnt/bench/matterweave-dev/performance/engine-02/async-coverage`.

## Engineering log

**Actions:** GLM5.3 Flash/high wrote the controller and tests in a bounded15-minute
attempt. Lead recovered the completed implementation from its saved file after
it timed out during a failed RED-stub setup. Astra medium repaired test-only
consumption/race assumptions in276s; it did not compile or run tests. Lead reviewed,
executed tests, fixed shutdown refusal and integrated the native Vulkan adapter.

**Issues:** The combined351-test workspace suite passed. Strict Clippy initially
caught the lead's test-only field reassignment; initializing the saturated counter
directly fixed it, and the focused test plus strict workspace Clippy passed.
GLM left unverified code and no checkpoint commits. Its original tests
consumed buffered results before checking them and asserted race-sensitive queue
counts. Those were repaired; worker completion alone was not treated as acceptance.
Astra's tests also covered open-room radiance and deterministic queue transitions.

**Insights:** CPU preparation now progresses without holding a Vulkan frame fence.
Visible direct-only waiting frames demonstrate scheduling behavior; temporal
quality, production cadence, full-scene cost and sustained mobile efficiency remain
open. The existing synchronous check remains available as a reference.
