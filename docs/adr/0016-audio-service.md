# ADR-0016: Bounded audio service with deterministic mixer and AAudio output

- Date: 2026-09-09
- Status: Proposed
- Basis: Engineering selection under accepted ADR-0014 for the M6 reusable-framework milestone; independent evaluation trial on branch `eval/glm-audio` from base `5b90975`.
- Requirements: R11, R13, R17, R18

## Context

M6 needs a small reusable audio capability that both engine samples can consume without NDK objects or real-time threading. The workspace already pins `ndk 0.9.0` (transitive via winit), which provides complete AAudio bindings with an explicit contract: callbacks are boxed `FnMut(+Send)` closures owned by the `AudioStream`, never invoked simultaneously, never after `AAudioStream_close` returns, and the error callback must not stop/close/reopen the stream itself. The frozen base had no audio service. An earlier unrelated attempt (`worktrees/engine-audio-service`, Hy4 trial) timed out and is preserved untouched; this trial is independent.

Acceptance limits (task limits, not owner-approved product guarantees): at most 32 registered clips, 8 simultaneous voices, 4 MiB retained PCM payload and 64 pending commands; mono/stereo f32 at 48 kHz; invalid channels, incomplete frames, nonfinite samples/gains and over-limit requests are rejected; voices are never stolen and commands never overwritten; output stays finite within [-1, 1]; the render callback performs no allocation, no I/O, no logging, no blocking locks and no stream shutdown.

## Decision

Add crate `matterweave-audio` with three layers.

**Mixer core** (render-thread state): fixed 32 clip slots, 8 voice slots, a separately shared, preallocated 4 MiB PCM pool that is never resized or freed while callbacks can access it, and a bounded SPSC command queue. Each render invocation applies the queued commands, finishes PCM reads, then publishes a completed-command sequence with release ordering so the control thread can prove application before reusing memory. Mixing is deterministic: voices in slot order; mono clips upmixed to both channels without attenuation; stereo clips passed through; mono devices take the mean of both contributions; samples clamped to [-1, 1] with a finite guard.

**Service** (game thread, `&mut self` API): generational `ClipHandle`/`VoiceHandle`, validate-then-mutate operations, explicit rejection errors (`ClipLimit`, `VoiceLimit`, `PcmBudgetExceeded`, `CommandQueueFull`, `RangeNotYetAcknowledged`, `StaleClipHandle`, `StaleVoiceHandle`, `InvalidGain`, format errors), negotiated stream properties and a health snapshot (callback/frame/command counts, rejection counters, silenced/completed voice counts, device xruns/errors, recreation count).

**Backends** behind a crate-private trait: a host `mock` backend driven synchronously by tests, and an Android `aaudio` backend over the pinned `ndk` bindings with `api-level-28` features; NDK types never appear in the public API. Open requests 48 kHz/f32/stereo and verifies negotiation, failing explicitly with `FormatRejected` otherwise. Device loss is recorded into shared atomics by the error callback; `poll_device` closes and reopens the stream around the same mixer core, so voices continue where they stopped.

Lifecycle and ownership policies (documented and tested):

- Clip payloads live in a private fixed `Arc<PcmPool>` of per-sample `UnsafeCell<f32>` shared by service and mixer. Registration writes only free ranges without accessing the live mixer; callbacks read only published ranges. A freed range is reused only after an acquire observes the completed unload command (voices silenced and final reads complete); while suspended no invocations occur, so registration can return `RangeNotYetAcknowledged` until resume.
- Unregistering a clip silences its voices; their handles become no-op targets for `stop_voice`.
- Suspend/resume are FIFO commands around device pause/start: the mixer clock freezes mid-sample, nothing is dropped or restarted, commands queued during suspension apply in order at resume, and a queued play that had not started begins after resume.
- Capacity decisions reflect render-thread applied state (completed play-command sequence + live mask), never a queued promise, so the voice limit cannot be oversold and a stop-then-play may need a retry after the next callback.
- Drop order closes the stream (joining all callbacks) before freeing the mixer core; the callback closures hold a raw pointer and drop with the stream object. `CoreOwner` owns the allocation without retaining a Box reference while callbacks run; it reconstructs the Box only after backend destruction.

Adopted dependencies: `ringbuf = 0.5.1` (MIT OR Apache-2.0, no unsafe user code, push/pop fail without overwriting/blocking) for the command queue, and the existing `ndk 0.9.0` (MIT OR Apache-2.0) for AAudio. No other new dependencies.

## Alternatives

- `cpal`/`oboe`/`rodio`/`kira`: general host-abstraction and streaming/music libraries bring larger dependency trees and out-of-scope codecs/graphs; the target is Android AAudio only. Rejected (R11: smallest adequate integration).
- Custom C audio bindings: unnecessary; the pinned `ndk` bindings already provide verified AAudio FFI with linked `libaaudio`. Rejected.
- Hand-rolled lock-free SPSC ring: rejected per reuse policy; `ringbuf` is an established audio real-time queue with try-semantics matching the requirements.
- Out-of-band suspend flag instead of FIFO commands: would reorder against play/stop commands and weaken the determinism guarantees. Rejected.

## Consequences

Games get sound effects through a small handle-based API with explicit backpressure; they own retry policies. No resampling and no compressed formats (this trial): clips must be 48 kHz f32, and a device that cannot negotiate 48 kHz f32 mono/stereo fails open explicitly. Output opens stereo when the device offers it. Real-time safety is enforced by tests (allocation detector, threaded mixer interleaving) and the structural ownership argument above; AAudio's close-joins-callbacks guarantee is relied on and cited, not re-derived. `AudioStream::drop` (ndk) unwraps the close result, so a failed stream close aborts — an accepted platform-boundary failure mode, documented in the crate.

## References

- [REQUIREMENTS](../REQUIREMENTS.md) R11, R13, R17, R18 and [ROADMAP](../ROADMAP.md) M6.
- [ADR-0014](0014-rust-modularity-and-evidence-led-reuse.md) (Rust, modularity, reuse policy) and [ADR-0015](0015-rust-native-foundation.md) (native Android foundation).
- [Component selection](../COMPONENT_SELECTION.md) procedure.
- ndk 0.9.0 source (`src/audio.rs`) callback and stream lifetime contracts; AAudio developer documentation for `AAudioStream_close` callback joining and error-callback threading rules.
- ringbuf 0.5.1 source (`try_push`/`try_pop` fetch-before-fail semantics).
- Initial trial report in `eval/glm-audio`; subsequent corrective reviews and device evidence are linked from [STATUS](../STATUS.md).

## Validation

The following initial-trial results are historical; the corrective checkpoint below supersedes their pending device status.

- Host: 22 tests cover the two-voice reference fixture within 1e-6, clipping, stereo order, completion, gain, stop, silence, limit enforcement at limit/limit+1 and after reuse, atomic rejection, stale handles, command saturation and recovery, the suspend/resume policy, fault-injected device loss with recreation, and a threaded mixer interleaving test. A dedicated allocation-detector binary reports zero allocations and deallocations across 10,000 callback invocations including completion and stop handling.
- Cross-compilation: the diagnostic example builds for `aarch64-linux-android` with the repository's pinned NDK 28.2.13676358 API-28 linker and links `libaaudio`; scoped strict Clippy passes for both host and Android targets.
- Pending device gate: run the diagnostic on a reserved phone (open output, negotiated format/rate, nonzero frames, suspend/resume, controlled recreation, ten open/play/stop/close cycles, callback/xrun counts). Not run in this trial because no device was attached; the executable, SHA-256 and procedure are delivered with the trial report, and final acceptance requires the coordinating reviewer to execute it.

### 2026-09-12 corrective checkpoint

Source `a999fd0` repairs suspension observation, paused output recovery, failed-pause
compensation, unload generation, completed-command lifetime acknowledgment, and
live-core PCM aliasing. Package tests: 49 PASS on host and as native ARM64 phone binaries; allocation detector covers
10,000 callback invocations. On OnePlus 13 / Android 16, the real AAudio diagnostic
passes four runs of 20 suspend/recreate/resume/shutdown cycles at that source
([manifest](../evidence/2026-09-12-completion-wave1/android-audio-final-manifest.json)).
Independent Muse/Gemini corrective reviews pass at this checkpoint. No human audibility,
actual headset-disconnect or Miri result is claimed. Proposed status remains: M6
both-sample service integration/reuse and broader acceptance are still open.

The ownership proof relies on narrow per-cell access and FIFO publication/retirement
ordering; `UnsafeCell` alone supplies neither synchronization nor permission for
conflicting mutable references. See the [Rust contract](https://doc.rust-lang.org/std/cell/struct.UnsafeCell.html)
and [AAudio thread-safety contract](https://developer.android.com/ndk/guides/audio/aaudio/aaudio).
