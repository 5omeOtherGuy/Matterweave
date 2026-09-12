Engineering log (22070ca..b58527f, audio crate only):

- Traced `AudioService::register_clip/unregister_clip/play/stop_voice` (`service.rs`), `MixerCore::render_with_after_drain/apply` (`mixer.rs`), `PcmPool::write/read` (`pcm.rs`), `CoreOwner/CorePtr` + `OutputBackend` (`backend.rs`, `backend/mock.rs`, `backend/aaudio.rs`), `command.rs` queue.
- Publication: pool `write` before FIFO `push`; mixer drain→mix→`live_mask`/`commands_applied` Release store. Retirement: control Acquire of `commands_applied` before PCM reuse/voice-slot trust (`reclaim_acknowledged_ranges`, `find_free_voice_slot`, `voice_definitely_finished`, `is_voice_active`). `UnsafeCell` correctly not claimed as sync; ack is the fence.
- Unload carries retired generation; mixer rejects mismatch. Suspend reserves 2 FIFO slots with compensating `Resume`; failure leaves device+mixer consistent. Recreation closes/joins before clearing `disconnected/error_code` and opening, preserving new-stream errors; `properties()` hides lost/disconnected.
- Core: `Box::into_raw` once, control never dereferences live core (only backend callbacks + synchronous mock); field order + `Drop` closes backend before `CoreOwner::from_raw`; pool is separate `Arc`, `&mut MixerCore` never borrows PCM pointee. Render performs no `Arc` clone/drop, allocation, lock, I/O.
- No mutable live-core borrow, allocation/locking violation, ack-ordering break, open/start/close-path leak, or teardown UAF proven from source in scope. Pinned `ringbuf`/`ndk` internals not unpacked; contracts taken as documented.

Findings: no findings valid.

NOT RUN native/Miri; lead owns reproductions and acceptance.

