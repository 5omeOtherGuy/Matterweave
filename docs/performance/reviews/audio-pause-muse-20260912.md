Review: `suspend` pause-intent retention during failed recovery — no remaining defects.

Trigger: `recreate_output_with` open/start failure leaves `running=false`, `backend=None`, `recovery_pending=true`, `suspended=false` (`crates/matterweave-audio/src/service.rs:684-696`; `start_output` failure path `:278-291` preserves pause/`Resume` intent, only clears `suspended` on success at `:292-294`). Old `suspend` guard `if !running return Ok` dropped intent here.

Fix verified at `service.rs:514-541`:
- Guard is now `if self.suspended return Ok` (`:515-517`), so failed-recovery (`running=false`, no stream) still queues `Suspend`, skips absent-backend device pause (`:530-537`), and sets `running=false, suspended=true` (`:539-540`). Later `poll_device` (`:575-590`) → `recreate_output_with` gated by `if !self.suspended { start }` (`:697-701`) opens but does not start — no unintended background start.
- Queue compensation sound: reserves 2 slots (`:524-529`), pushes `Suspend` (`:528-529`), compensating `Resume` on device-pause failure (`:534-537`); single-producer invariant (`:522-523`) holds, so compensation cannot overflow.
- Idempotency preserved: already-`suspended` early-return touches no queue/`resume_queued`; `resume` idempotent while `running` (`:549-551`) and `resume_queued` guard (`:560-563`) prevents duplicate `Resume` across failed starts — matches `pending_commands` unchanged assertion.
- No stale `Resume` bypass: fresh `suspend` clears `resume_queued=false` (`:541`) so a later `resume` queues a new `Resume` (`:560-566`, before `start_output`, FIFO `Suspend→Resume` for PCM continuation); already-suspended path returns before `:541`, retaining the failed-resume's queued `Resume`. `start_output` success clears both flags (`:292-294`).

New tests cover open-fail and start-fail branches plus suspend-after-failed-resume idempotency/continuation.

No edits/builds. Device statement: NOT RUN on device (host read-only review, no APK/device execution).

