# ADR-0016: Bounded audio service with deterministic mixer and AAudio output

- Date: 2026-09-09
- Status: Proposed
- Basis: Engineering selection under accepted ADR-0014 for the M6 reusable-framework milestone; independent evaluation trial on branch `eval/glm-audio` from base `5b90975`.
- Requirements: R11, R13, R17, R18

## Context

M6 needs a small reusable audio capability that both engine samples can consume
without handling NDK objects or real-time threading themselves. The workspace
already pins `ndk 0.9.0` (transitive via winit), whose source contains complete
AAudio bindings with an explicit callback/lifetime contract: callbacks are boxed
`FnMut(+Send)` closures owned by the `AudioStream` object, never invoked
simultaneously, never invoked after `AAudioStream_close` returns, and the error
callback must not stop/close/reopen the stream itself. The frozen base had no
audio service. An earlier unrelated attempt (`worktrees/engine-audio-service`,
Hy4 trial) timed out and is preserved untouched; this trial is independent.

Acceptance limits for the service (task acceptance limits, not owner-approved
product guarantees): at most 32 registered clips, 8 simultaneous voices, 4 MiB
retained PCM payload, 64 pending commands; mono/stereo f32 at 48 kHz; invalid
channels, incomplete frames, nonfinite samples/gains and over-limit requests are
rejected; voices are never stolen and commands are never overwritten; output
stays finite within [-1, 1]; the render callback performs no allocation, no
I/O, no logging, no blocking locks and no stream shutdown.

## Decision

Add crate `matterweave-audio` with three layers:

1. **Mixer core** (render-thread state): fixed arrays of 32 clip slots and 8
   voice slots, one preallocated PCM pool sized to the 4 MiB budget that is
   never resized or freed while the service lives, and a bounded SPSC command
   queue. Every state mutation is a command applied at the start of a render
   invocation; the invocation ends by publishing an ack epoch, so the control
   thread can prove a command was applied before reusing memory. Mixing is
   deterministic: voices in slot order, mono clips upmixed to both channels
   without attenuation, stereo clips passed through, mono devices take the mean
   of both contributions, samples clamped to [-1, 1] with a finite guard.
2. **Service** (game thread, `&mut self` API): generational `ClipHandle`/
   `VoiceHandle`, complete-validate-then-mutate operations, explicit rejection
   errors (`ClipLimit`, `VoiceLimit`, `PcmBudgetExceeded`, `CommandQueueFull`,
   `RangeNotYetAcknowledged`, `StaleClipHandle`, `StaleVoiceHandle`,
   `InvalidGain`, format errors), negotiated stream properties and a health
   snapshot (callback/frame/command counts, rejection counters, silenced and
   completed voice counts, device xruns/errors, recreation count).
3. **Backends** behind a crate-private trait: a host `mock` backend driven
   synchronously by tests, and the Android `aaudio` backend using the pinned
   `ndk` bindings with `api-level-28` features. NDK types never appear in the
   public API. Open requests 48 kHz/f32/stereo and verifies the negotiation,
   failing explicitly with `FormatRejected` otherwise. Device loss is recorded
   by the error callback into shared atomics; `poll_device` closes and reopens
   the stream around the same mixer core, so voices continue where they
   stopped.

Lifecycle and ownership policies (documented and tested):

* Clip payloads always live inside the service-owned pool, so render reads
  cannot hit freed memory. A freed range is reused only after the ack epoch
  proves the unload was applied (voices silenced). While the output is
  suspended no invocations occur, so such reuse is unavailable and
  registration can return `RangeNotYetAcknowledged` until resume.
* Unregistering a clip silences all voices playing it; their handles become
  no-op targets for `stop_voice`.
* Suspend/resume are FIFO commands around device pause/start: the mixer clock
  freezes mid-sample, nothing is dropped or restarted, commands queued during
  suspension apply in order at resume, and a queued play that had not started
  begins after resume.
* Capacity decisions reflect render-thread applied state (play command ack
  epoch + live mask), never a queued promise, so the voice limit cannot be
  oversold and a stop-then-play may need a retry after the next callback.
* Drop order closes the stream (joining all callbacks) before freeing the
  mixer core; the callback closures own the core pointer and are dropped with
  the stream object.

Adopted dependencies: pinned `ringbuf = 0.5.1` (MIT OR Apache-2.0, no unsafe
user code, push/pop fail without overwriting/blocking) for the command queue;
existing `ndk 0.9.0` (MIT OR Apache-2.0) for AAudio. No other new dependencies.

## Alternatives

- `cpal`/`oboe`/`rodio`/`kira`: general host-abstraction or streaming/music
  libraries bring larger dependency trees and out-of-scope codecs/graphs; the
  production target is Android AAudio only. Rejected for this scope (R11:
  smallest adequate integration).
- Custom C audio bindings: unnecessary; the pinned `ndk` bindings already
  provide verified AAudio FFI with linked `libaaudio`. Rejected.
- Hand-rolled lock-free SPSC ring: rejected per reuse policy; `ringbuf` is an
  established audio real-time queue with try-semantics matching the
  requirements.
- Out-of-band suspend flag instead of FIFO commands: would reorder against
  play/stop commands and weaken the determinism guarantees. Rejected.

## Consequences

Games get sound effects through a small handle-based API with explicit
backpressure; they own retry policies. No resampling and no compressed formats
(this trial): clips must be 48 kHz f32, and a device that cannot negotiate
48 kHz f32 mono/stereo fails open explicitly. Output opens stereo when the
device offers it. Real-time safety is enforced by tests (allocation detector,
threaded mixer interleaving) and by the structural ownership argument above;
AAudio's close-joins-callbacks guarantee is relied on and cited, not re-derived.
`AudioStream::drop` (ndk) unwraps the close result, so a failed stream close
aborts — an accepted platform-boundary failure mode, documented in the crate.

## References

- [REQUIREMENTS](../REQUIREMENTS.md) R11, R13, R17, R18 and [ROADMAP](../ROADMAP.md) M6.
- [ADR-0014](0014-rust-modularity-and-evidence-led-reuse.md) (Rust, modularity,
  reuse policy) and [ADR-0015](0015-rust-native-foundation.md) (native Android
  foundation).
- [Component selection](../COMPONENT_SELECTION.md) procedure.
- ndk 0.9.0 source (`src/audio.rs`) callback and stream lifetime contracts;
  AAudio developer documentation for `AAudioStream_close` callback joining and
  error-callback threading rules.
- ringbuf 0.5.1 source (`try_push`/`try_pop` fetch-before-fail semantics).
- Trial report and acceptance matrix in the `eval/glm-audio` pull request;
  device run pending (see [STATUS](../STATUS.md)).

## Validation

- Host: 22 tests cover the two-voice reference fixture within 1e-6, clipping,
  stereo order, completion, gain, stop, silence, limit enforcement at
  limit/limit+1 and after reuse, atomic rejection, stale handles, command
  saturation and recovery, the suspend/resume policy, fault-injected device
  loss with recreation, and a threaded mixer interleaving test. A dedicated
  allocation-detector binary reports zero allocations and deallocations across
  10,000 callback invocations including completion and stop handling.
- Cross-compilation: the diagnostic example builds for `aarch64-linux-android`
  with the repository's pinned NDK 28.2.13676358 API-28 linker and links
  `libaaudio`; scoped strict Clippy passes for both host and Android targets.
- Pending device gate: run the diagnostic on a reserved phone (open output,
  negotiated format/rate, nonzero frames, suspend/resume, controlled
  recreation, ten open/play/stop/close cycles, callback/xrun counts). Not run
  in this trial because no device was attached; the executable, SHA-256 and
  procedure are delivered with the trial report. Final acceptance requires the
  coordinating reviewer to execute it.
