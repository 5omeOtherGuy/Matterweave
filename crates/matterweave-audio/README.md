# matterweave-audio

Bounded PCM sound-effect service: a game-facing handle API, a deterministic mixer built for
the real-time render callback, and a native Android AAudio output backend.

## What it provides

`AudioService` owns one mixer core, one bounded command queue and one output stream. The game
thread calls it through a `&mut self` control API; the render thread only runs `MixerCore`
through the backend's data callback. Backend and real-time types never cross the public API.

| Layer | Responsibility |
| --- | --- |
| `AudioService` | Register/play/stop/gain, suspend/resume, device recovery, counters and properties. |
| `MixerCore` (crate-private) | Fixed clip/voice slots, one preallocated PCM pool, command application and sample mixing. |
| Backend (crate-private) | `mock` for deterministic host tests, `aaudio` for device output through the pinned `ndk` 0.9.0 bindings. |

## Public surface

| Item | Notes |
| --- | --- |
| `AudioService` | `new`; `register_clip`, `unregister_clip`, `play`, `stop_voice`, `set_voice_gain`; `suspend`, `resume`, `poll_device`; `stream_properties`, `clip_is_registered`, `is_voice_active`, `health`, `retained_pcm_bytes`; `SAMPLE_RATE`; `mock_backend`, `mock_render`; `diagnostic_recreate_output`. |
| `ClipSpec` | `samples`, `channels`, `mono`, `stereo`, `SAMPLE_RATE`. |
| `ClipHandle`, `VoiceHandle` | `slot`, `generation`, `Display`; copyable and generational. |
| `PlayOptions` | `gain`, `with_gain`, default gain 1.0. |
| `StreamProperties` | Negotiated rate, channels, float format, burst/buffer sizes, device/session ids, low-latency and exclusive flags. |
| `HealthSnapshot` | Callback/frame/command counters, rejection and silence/completion counters, xruns, device errors, recreations, ack epoch, service suspension (`suspended`) and the render-thread mirror (`rt_suspended`). |
| `AudioServiceError`, `InvalidClipReason`, `DeviceError` | Plain data; no backend types. |
| `AudioLimits` | `pcm_pool_samples()`; plus `MAX_CLIPS`, `MAX_VOICES`, `MAX_PCM_BYTES`, `MAX_COMMANDS`. |
| `config` module | `SAMPLE_RATE` (48 000) and `MAX_GAIN` (8.0) alongside the limit constants. |

Cargo features select the backend:

- `backend-mock` (default): the host backend driven synchronously by `MockOutput::render`
  through `AudioService::mock_render`. Required for host tests and the allocation detector; it
  proves nothing about device behavior.
- `backend-android`: real output through `ndk` AAudio. Device builds use
  `--no-default-features --features backend-android`.
- `diagnostic`: adds `diagnostic_recreate_output` for the controlled close/reopen used by the
  diagnostic example.

## Invariants and guarantees

### Enforced limits

| Resource | Limit |
| --- | --- |
| Registered clips | 32 (`MAX_CLIPS`) |
| Simultaneously active voices | 8 (`MAX_VOICES`) |
| Retained PCM payload | 4 MiB (`MAX_PCM_BYTES`, 1 048 576 f32 samples) |
| Pending control commands | 64 (`MAX_COMMANDS`) |

The clip pool is allocated once at service creation at exactly `MAX_PCM_BYTES` and is never
resized. Operations that would exceed a limit fail explicitly; voices are never stolen and
queued commands are never overwritten. A full command queue returns `CommandQueueFull`
(backpressure); the game owns the retry policy.

### Input and output contracts

- Clips are interleaved mono or stereo f32 at 48 kHz (`ClipSpec`, `AudioService::SAMPLE_RATE`).
  A clip is validated completely before any state changes: bad channel count, empty clip,
  incomplete frame or nonfinite sample rejects atomically.
- The output stream is requested as 48 kHz, f32, stereo. Negotiated properties are verified;
  a stream that cannot provide 48 kHz f32 mono or stereo is closed again and reported as
  `FormatRejected` instead of misinterpreting bytes.
- Mixed samples are finite and within `[-1.0, 1.0]`. Mono clips are upmixed to both output
  channels without attenuation, stereo clips pass through, and a mono device receives the mean
  of both contributions.

### Real-time and lifetime guarantees

- One render invocation performs no heap allocation or deallocation, no file or network I/O,
  no logging, no blocking locks and no stream shutdown. Control commands travel through a
  bounded `ringbuf` SPSC queue and are applied in FIFO order at the start of an invocation.
- Clip payloads live in the service-owned pool for the service lifetime, so a render read can
  never touch freed memory.
- `ClipHandle` and `VoiceHandle` are generational. Unregistering a clip or reusing a voice slot
  invalidates older handles; a stale handle is rejected and cannot affect the slot's new owner.
- Unregistering a clip silences every voice still playing it (rather than detaching it). The
  freed PCM range is reused only after the render thread has acknowledged the unload with its
  ack epoch.
- Suspend freezes the mixer clock mid-sample: nothing is dropped or restarted, and commands
  queued during suspension are applied in FIFO order on resume. Resume queues the command
  before restarting the stream so there is no silent gap. `health().suspended` reports the
  service state as soon as `suspend()`/`resume()` returns, without waiting for a render
  callback. `health().rt_suspended` is the render thread's own view; on a backend that
  delivers no callbacks while paused it may remain false. AAudio pause is asynchronous;
  service suspension is not proof that in-flight callbacks have finished.
- Device loss is recorded by the AAudio error callback into shared atomics. `poll_device`
  closes and reopens the stream around the same mixer core, so voices continue where they
  stopped. Suspended replacements remain unstarted until `resume()`, including diagnostic
  recreation. Failed open/start leaves recovery retryable by `poll_device()`; failed resume
  retains suspension until `resume()` succeeds, without queuing duplicate Resume commands.
- `Drop` closes the stream before dropping the mixer core. AAudio's `AAudioStream_close` joins
  all callback threads before returning, so no callback can observe freed state.

## Limits and what it does not do

- Gain must be finite and within ±8.0 (`InvalidGain`); gain is a linear scalar.
- `play` capacity reflects render-thread applied state, not queued promises: a play
  immediately after a stop may be rejected with `VoiceLimit` until the next callback applies
  the pending commands. Retry after the next callback on a live device.
- A suspended service produces no render invocations, so freed PCM ranges stay pending: a
  registration that needs that memory returns `RangeNotYetAcknowledged` until resume.
- The only termination path for a failed stream close is the `ndk` wrapper's `AudioStream`
  drop, which unwraps the close result and aborts. This is an accepted platform-boundary
  failure mode.
- `AudioStream` is a single control-thread object and `AudioService` is `Send` but not `Sync`;
  the API is designed for one controlling thread.

### Not implemented

[ADR-0016](../../docs/adr/0016-audio-service.md) also names capabilities not built here:

- Resampling and compressed audio formats. Clips must be 48 kHz f32 mono/stereo, and a device
  that cannot negotiate that fails open explicitly.
- An on-device run: [STATUS](../../docs/STATUS.md) records the reserved-device diagnostic as
  NOT RUN. The Android backend compiles for `aarch64-linux-android` and links `libaaudio`, but
  on-device audibility, format negotiation and real device-loss recovery are unverified.
  ADR-0016 is Proposed.

## How it is tested

From the repository root:

```sh
cargo test -p matterweave-audio --features backend-mock
```

- `tests/acceptance.rs` (21 tests) covers the two-voice fixture against an independent
  reference within 1e-6, clipping and finiteness, stereo order, mono upmix, gain, completion,
  stop, silence, limit enforcement at the limit and limit+1 and after reuse, atomic clip
  rejection, stale handles, command-queue saturation and recovery, the suspend/resume
  continuation policy (service truth observable without a render callback, render mirror
  separate) and fault-injected device loss with recreation.
- `tests/recovery.rs` (4 tests) covers device-loss and diagnostic recreation with applied
  and buffered Suspend, no callbacks while paused, and exact PCM continuation.
- `src/service/tests.rs` (5 tests) checks that paused replacements never request start,
  failed open/start retries, bounded Resume command retries and queue-full recovery.
- `tests/rt_allocations.rs` (1 test) is a dedicated allocation detector that reports zero
  allocations and deallocations across 10 000 callback invocations, including completion and
  stop handling.
- One inline mixer unit test interleaves a real render thread with a full control-command
  push stream and asserts no panic or deadlock, full command application and sane
  diagnostics.

The host diagnostic example (`examples/audio_diagnostic.rs`) exercises the mock backend:
negotiated properties, frame progress, suspend/resume, controlled recreation and ten
open/play/suspend/recreate-while-suspended/resume/stop/close cycles. Each progress check has
an asserted two-second deadline. Paused counters are checked only after recreation has
closed the old stream, not immediately after the asynchronous pause request. It is silent by design and is not device evidence. The executed
host and cross-compilation results are recorded in [STATUS](../../docs/STATUS.md) and
[ADR-0016](../../docs/adr/0016-audio-service.md).
