### Statement on Verification Executions
**NOT RUN**: Native builds, target executions (Android AAudio/NDK), and Miri executions were **NOT RUN** in this independent review leaf. Lead owns reproductions and acceptance.

---

### Source-Proven Defects

#### 1. `suspend()` returns `Ok(())` without setting `suspended = true` when called while stream is not running
- **File & Line**: `crates/matterweave-audio/src/service.rs:512`
- **Concrete Trigger**:
  1. The output stream experiences device loss, open failure, or start failure during initialization or recovery, transitioning to `self.running == false`, `self.recovery_pending == true`, and `self.suspended == false`.
  2. Caller invokes `service.suspend()` (e.g. application transitions to background or pause state).
  3. Line 512 evaluates `if !self.running { return Ok(()); }` and immediately returns `Ok(())` without setting `self.suspended = true`.
- **Consequence**:
  1. `service.health().suspended` reports `false`, violating the documented health contract ("*Service suspension truth: `true` after a successful `AudioService::suspend` ... Preserved across recreation and failed recovery, even while no stream is open*").
  2. When the device recovers or `service.poll_device()` succeeds, `recreate_output_with` evaluates `if !self.suspended` (which is `true`) and calls `start_output()`. The engine begins active audio playback in the background despite the caller having successfully invoked `suspend()` and never calling `resume()`.
- **Uncertainty**: None. The early return at line 512 conflates "not currently rendering" with "service suspended", preventing suspension intent from being recorded whenever the stream is stopped or recovering. (The guard should test `if self.suspended { return Ok(()); }`).

---

### Concise Engineering Log

1. **Command-Completion Acknowledgement before PCM/Voice Reuse**:
   - `AudioService::unregister_clip` tags retired ranges with `self.commands_pushed` (the 1-indexed FIFO sequence of `UnloadClip`).
   - In `MixerCore::render_with_after_drain`, `self.commands_applied` increments on command pop, voices for the unloaded clip are marked `active = false` before mixing, mixing completes all PCM reads, and `commands_applied` is published with `Ordering::Release`.
   - `AudioService::reclaim_acknowledged_ranges` loads `commands_applied` with `Ordering::Acquire` before moving ranges to `free_ranges`.
   - For voice reuse, `find_free_voice_slot` gates voice slot reuse on `ack >= mirror.play_sequence && !self.rt_voice_live(index)`, preventing stale live bits from causing premature voice allocation.

2. **Unload Generation**:
   - Fixed in `audio-final.diff`: `UnloadClip` now carries `generation: clip.generation` instead of `next_generation(clip.generation)`.
   - `MixerCore::apply(Command::UnloadClip)` checks `rt_slot.active && rt_slot.generation == generation`. Because `LoadClip` registered `generation`, this check succeeds and silences voices. Stale unloads for older generations are rejected without silencing current voices.

3. **Pause-Failure Queue Compensation**:
   - `suspend()` checks `queue::vacant(&self.producer) < 2` upfront, reserving both the `Suspend` command and potential compensating `Resume`.
   - If `backend.suspend()` fails, `Command::Resume` is queued into the reserved slot and `Err(device_error)` is returned, keeping device and mixer core states synchronized.

4. **Stale/New Device-Error Recovery and Negotiated Properties**:
   - `AaudioOutput::properties()` and `MockOutput::properties()` now check `self.lost || self._shared.disconnected.load(Ordering::Acquire)`, hiding properties immediately when the error callback fires before `poll_device()` runs.
   - `recreate_output_with` resets `self.shared.disconnected` and `self.shared.error_code` *after* joining old callbacks via `backend.close()?` and *before* calling `open(...)`. If the new stream fires an error callback during or after open, its notification is preserved rather than erased.

5. **Separately Shared PCM Cells and Raw CoreOwner Lifetime**:
   - `PcmPool` encapsulates `Box<[UnsafeCell<f32>]>` behind `Arc<PcmPool>`. Interior mutability uses `UnsafeCell::get().write()` and `read()` by value; neither `&f32` nor `&mut f32` references are ever formed.
   - `CoreOwner` wraps raw `*mut MixerCore` via `Box::into_raw`. No `Deref` or mutable references to `MixerCore` are created from the control thread while callbacks are wired.
   - `AudioService::drop` sets `self.backend = None` first, invoking `AAudioStream_close` which joins all callback threads per official Android contract. `CoreOwner::drop` only reconstructs `Box<MixerCore>` after callbacks have joined.

6. **Public Safe API Concurrency and Invariants**:
   - Public methods (`register_clip`, `play`, `unregister_clip`, `stop_voice`, etc.) interact with the audio callback solely through atomic flags, the lock-free SPSC queue (`ringbuf`), and non-overlapping PCM cells.
   - The render callback adheres strictly to real-time safety invariants: zero allocations, zero deallocations, zero blocking locks, and zero I/O.
   - Moving `AudioService` transfers heap pointers/Arc owners without moving the heap-allocated `MixerCore`, keeping the callback's `CorePtr` valid.
