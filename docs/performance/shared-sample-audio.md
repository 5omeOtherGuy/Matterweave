# Shared sample audio and Android recovery

The Android chooser owns one audio adapter across wetland and Voxel Relay.
Actual wetland jump/landing/edit and Relay obstacle/objective events feed bounded
queues, with scope retirement and lifecycle suspension. Direct standalone CLI
sample modes remain silent. Relay chooser saves now use `voxel-relay.json`,
importing an existing legacy puzzle once without overwriting `world.json`.

Physical testing found and repaired a missed integration failure: after HOME,
AAudio refused the old stream start with -899. The service correctly retained
suspension, but the app never retried its resume. The adapter now retries failed
foreground resumes with a 500 ms cooldown, cancels on intentional suspension,
and drops new events while output is unavailable. The mock's one-shot start
failure exercises the real service failure/recreation path.

## Verification

Source `b2ffd5b4c5b68ea10ab306c24b112135a787e7af`; [APK identity and conditions](../evidence/2026-09-12-shared-audio/manifest.json).
OnePlus13 Android16: **3/3 wetland HOME/resume cycles** reproduced -899, then
started replacement streams successfully and submitted jump/landing events with
increasing callbacks/frames without a second lifecycle resume. Voxel Relay also
recovered from -899 and submitted its actual obstacle-clear event afterwards.
[Before/after runtime evidence](../evidence/2026-09-12-shared-audio/runtime.log).

The Relay test placed the character near the obstacle through a disposable save,
then invoked the real ACTION. A separate pre-repair run of unchanged event wiring
prepositioned the crate on the plate and observed the real objective transition.
These are fixture-driven integration checks, not a claim that a human played the
whole puzzle. The completed legacy puzzle imported correctly into the dedicated
save. Original wetland/world/recovery saves and completed Relay progress were
restored; [SHA-256 evidence](../evidence/2026-09-12-shared-audio/restored-saves.sha256).

Host:147 app tests pass (one ignored),49 audio tests pass, scoped strict Clippy
and fmt pass. The three recovery tests discriminate missing retry, missing output
guard and missing cancellation. [Worker log and exact commands](logs/audio-resume-repair.md).
Independent Gemini review `w_3e710340` of the repair found no defects; lead checked
the service contracts and ran the phone reproducer. Earlier integration reviews
`w_bc601816` and `w_f715246c` found no defects but missed the Android failure;
hardware testing supplied the decisive evidence.

**Human audibility remains unverified.** Callback counts and mock PCM do not prove
speaker output. A listening question is pending; this code slice does not close M6,
physical multitouch/lock-unlock, mute/volume UI, or the engine's remaining gates.
No performance or thermal claim is made.

Reproduce: build/install per [DEVELOPMENT](../DEVELOPMENT.md), enter wetland,
tap JUMP, HOME for3seconds, resume for7seconds, tap JUMP again; repeat3times.
Observe new successful AAudio starts and increasing event callback/frame counters.
Preserve existing saves before fixture manipulation. Raw device logs, disposable
fixtures and captures reside under `/mnt/bench/matterweave-dev/coarse-terrain/android-audio/`.
