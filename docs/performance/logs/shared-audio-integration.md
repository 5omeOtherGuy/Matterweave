# Engineering log w_78819ec8

Worker: `deepseek-flash-go` · outcome: Category: implement · run: `/mnt/bench/matterweave-dev/coarse-terrain/shared-audio`

Base `c095047`, branch `engine/shared-audio-integration`. Scope: integrate the merged
D4.1 `AudioAdapter` into the actual wetland and Voxel Relay samples. Owned paths only:
`apps/explorer/src/audio_service.rs`, `experience.rs`, `voxel_relay.rs`, `wetland.rs`
(gameplay-event emission and bounded event access only) and this log.

- **Actions Taken:** One `AudioAdapter` is now owned by `Experience` and drains
  bounded plain `GameplayEvent` queues from whichever sample is active. Focus and
  lifecycle intent were added after the lead's review finding. No sample owns a
  service or opens a device. Details under **Implemented**.
- **Issues & Friction:** The lead stopped the expensive duplicate dependency build
  in this worktree (`cargo test -p matterweave-explorer --lib --no-run`) so the
  integrated warm target is used once; per instruction no further Cargo command was
  run and execution is delegated to the lead. The wetland full-map gate is
  `#[ignore]`d, so a small real `Runtime` fixture was added to cover the real edit
  path without generating the full showcase twice.
- **Decisions & Rationale:** Focus loss is treated like backgrounding: queued events
  are dropped, sounding voices are *stopped* (not frozen to resume mid-clip) and the
  adapter is suspended. `focused` and `resumed` are tracked separately; focus gain
  never resumes a lifecycle-suspended app, and the pump only runs when both hold.
  Mute/volume have no existing UI seam, so no affordance was invented. Standalone
  sample launch stays explicitly silent: its bounded queue is never drained instead
  of opening a second device.
- **Solutions Applied:** Bounded `EventQueue` (capacity 16, drop-oldest) in
  `audio_service.rs`; sample accessors `pop_event`/`clear_events`; emission mapping
  to the existing vocabulary only (wetland: `BlockEdit`, `Jump`, `Land`; relay:
  `BlockEdit`, `Objective`); per-frame `pump_audio`; failure logging once per
  counter change; scope switch retires the leaving sample's events and voices before
  the arriving sample can play.
- **Insights:** The adapter's tracked-voice table plus `stop_all` is enough to make
  focus loss silence immediately while `suspend` keeps new events out; the two
  conditions (`resumed`/`focused`) must be owned by `Experience`, because the
  adapter cannot know why it was suspended. The wetland fixture shows the real edit
  path can be exercised without the full map if `Terrain::generate` is the only
  heavy dependency (accepted once per focused test).

## Implemented

- `audio_service.rs`: obsolete "Not wired yet" section and module-wide
  `#![allow(dead_code)]` removed. Added bounded `EventQueue` and narrow
  `AudioAdapter::is_suspended`. Docs now state that `Experience` owns the one
  adapter, that a directly launched sample is silent because its queue is never
  drained, and that mute/volume remain an explicit next action (targeted
  `#[allow(dead_code)]` on those two methods with the reason).
- `experience.rs`: owns `AudioAdapter`; `resumed` opens the device only when the
  window is focused; `suspended` stops voices, suspends and clears queues;
  `pump_audio` polls the device and drains the active sample once per frame;
  scope switches call `switch_to` after clearing the leaving sample's queue;
  adapter failures are logged at `warn` once per counter change and stay nonfatal;
  `exiting` stops all voices.
- Focus finding (lead): previously focus loss cleared queued events but left
  sounding voices running and `pump_audio` accepted events while unfocused. Now
  `on_focus_lost` clears queues, stops every tracked voice and suspends; focus gain
  resumes only when `resumed` is true; `pump_audio` requires both `resumed` and
  `focused` (and the adapter not suspended).
- `voxel_relay.rs`: bounded queue; `BlockEdit` on one successful obstacle removal;
  `Objective` once on the pressure-plate/door transition and once on the exit
  transition; `reset`/`load` clear the queue; `pop_event`/`clear_events` accessors.
- `wetland.rs` (emission/access only): bounded queue; `BlockEdit` on an accepted
  `Runtime::edit`; `Jump` only when a requested jump actually left the ground;
  `Land` only on an airborne-to-grounded step transition. No footstep cadence was
  invented and nothing triggers per frame. The existing ignored full-map gate now
  asserts exactly one `BlockEdit` through `WetlandApp::action`.
- Host tests added (execution delegated to the lead):
  - `audio_service::tests::event_queue_is_bounded_and_keeps_the_newest_events`
  - `experience::tests::no_device_before_resume_and_one_adapter_across_lifecycle`
  - `experience::tests::pump_plays_active_sample_events_and_suspension_drops_them`
  - `experience::tests::focus_loss_silences_and_focus_gain_cannot_override_lifecycle_suspension`
  - `experience::tests::scope_switch_drops_leaving_events_and_retires_its_voices_first`
  - `voxel_relay::tests::gameplay_events_follow_real_transitions_once`
  - `voxel_relay::tests::clear_and_reset_drop_queued_events`
  - `wetland::integration_tests::real_edit_queues_one_block_edit_and_rejections_queue_nothing`
  - `wetland::integration_tests::locomotion_events_follow_real_ground_transitions_only`

## Verification

- `rustfmt --edition 2021 apps/explorer/src/{audio_service,experience,voxel_relay,wetland}.rs`
  and `rustfmt --edition 2021 --check ...`: PASS (`FMT_OK`).
- Cargo host tests / Clippy / Android build / device audibility: **NOT RUN**.
  Reason: the lead stopped the duplicate dependency build to run the warm
  integrated target once; the lead owns test, review and Android execution. No
  compile, test or audibility result is claimed by this worker.

## Direct launcher gaps (lib.rs is lead-owned)

- `--voxel-relay` and `--sandbox` construct their sample directly and are silent by
  construction: the bounded queue is never drained and no device is opened. Android
  and the default/`--showcase` path go through `Experience` and play. If audio is
  wanted in the direct launcher, the wiring belongs in `lib.rs`.
- `docs/STATUS.md` still says "Audio is not yet wired into the samples; M6
  shared-service integration remains open." This commit makes that sentence
  obsolete; STATUS is lead-owned and was not edited.
- Mute/volume are implemented in the adapter but intentionally have no UI seam; the
  settings affordance is the next action, not a claimed integration.
