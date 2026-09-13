# Engineering log shared-settings-ordering-repair

Repair slice: finding 1 of the independent review in
`../orchestration/opus-settings-review/result.json` ("a pending preference change
is dropped if a sample switch is processed before `sync_settings`"). Branch
`engine/shared-mobile-settings`, frozen base
`560df252c17069d1ef186b52c0ee08f4eae38385` (open PR #53). No phone, no emulator,
no APK, no benchmark and no `/mnt/bench` work was performed.

Owned writes used: `apps/explorer/src/experience.rs` and this log. No other file
was changed: no sample (`wetland.rs`, `voxel_relay.rs`, `terrain_lab.rs`), no
`settings.rs`/`controls.rs`/`audio_service.rs`, no crate, no save schema, no
`docs/STATUS.md`, no `docs/HANDOFF.md`, no performance board, no `Cargo.lock`,
no CI.

## Actions Taken

1. Read the finding and `apps/explorer/src/experience.rs`, then confirmed the
   concrete trigger in source. `switch_if_requested` (`experience.rs:220`) runs
   inside `window_event`, while `sync_settings` (`:195`) only runs in
   `about_to_wait`. Chooser path: one event batch containing a settings-row
   toggle (`settings_dirty = true`), DONE (`settings_open = false`) and a
   sample-launch tap (`voxel_relay_requested = true`) reaches
   `switch_if_requested` before any `about_to_wait`; the branch retires the
   leaving scope and `self.wetland.take()` discards the one-shot change. Relay
   path: the same batch is toggle, DONE, MENU zone (`return_to_menu = true`),
   and `self.voxel_relay.take()` discards it. Result in both: no file write, no
   `AudioAdapter::set_muted`, and the arriving sample installed from a stale
   `self.settings`.
2. Added a named switch prelude `begin_scope_switch` (`:108`) that flushes
   pending preferences with `sync_settings()` and then retires the audio scope,
   and routed all five switch sites through it so the flush always happens
   before any sample is retired or taken.
3. Wrote `experience::tests::same_batch_preference_change_survives_the_scope_switch_without_about_to_wait`
   (`:655`): a real HUD batch (SETTINGS zone -> CONTROL SIZE toggle -> SOUND
   toggle -> DONE -> MENU zone) followed by the production prelude, with no
   `about_to_wait` and no test-side `sync_settings`, then assertions on owner
   state, persisted file, adapter mute/scope and the arriving sample's rendered
   layout.
4. Ran the red step (flush disabled) and then the green step, plus scoped
   `clippy`, `fmt --check` and `check_docs.py`; recorded exact commands and
   results under Verification.

## Issues & Friction

- **`switch_if_requested` is not reachable from a unit test.** `winit`'s
  `ActiveEventLoop` has no public constructor, so a regression test cannot call
  the function that contains the ordering bug. The flush therefore had to live
  at a point the test can drive; the chosen prelude is the first statement of
  every switch branch and runs immediately before retirement and `take`.
- **The reported scenario uses the wetland chooser, whose settings clicks are
  private to `wetland.rs`** and outside this slice's ownership. The relay
  leaving path drives the identical owner code (`sync_settings` ->
  `retire_leaving_scope` -> `take`), and its settings overlay is reachable
  through the public HUD path already used by `experience` tests, so the
  regression drives that path and observes the same `Experience` transition.
- **The arriving sample's preferences have no read accessor.** Wetland and relay
  keep `settings` private. The test installs the arriving relay with the same
  `set_shared_settings(self.settings)` call every production branch uses and
  observes propagation behaviorally through the configured move zone
  (`InputService::move_zone`), which changes only when the toggled
  `large_controls` preference reached the sample.
- The review's orchestration `result.json` reports the previous log as
  "unfilled: only 0/5 headings have content"; this log uses the conventional
  heading names the harness heuristic expects.

## Decisions & Rationale

- **Flush in a named switch prelude, not inside `retire_leaving_scope`.** The
  repair must not be forgettable and must be testable. `retire_leaving_scope`
  keeps its audio-only contract (drop events, stop voices); `begin_scope_switch`
  encodes the ordering "preferences first, then retirement" in one place, and
  every branch already calls it before `take()`.
- **Equivalent to, but narrower than, the reviewer's suggested placement.**
  Calling `sync_settings()` at the top of `switch_if_requested` would flush on
  every `window_event`, including events that request no switch, and would still
  be untestable without an `ActiveEventLoop`. `begin_scope_switch` flushes
  exactly when a switch is about to happen, which is the only moment at which a
  pending change can be destroyed by a `take()`.
- **No change to the one owner / one-shot change queue.** Samples still only
  set `settings_dirty` and expose `take_settings_change`; `Experience` remains
  the only writer of the file and the only caller of `AudioAdapter::set_muted`.
  `sync_settings` still short-circuits when there is no change and when the
  change equals the owner copy, so the fix adds no per-frame disk traffic.
- **Audio ordering preserved.** The flush runs before `clear_sample_events` and
  `audio.switch_to(arriving)`; `set_muted` is idempotent and `switch_to`
  (`audio_service.rs:560`) does not touch mute, so the leaving sample's voices
  are still retired first and the arriving scope inherits the restored policy.

## Solutions Applied

- `experience.rs:108` — new `begin_scope_switch(arriving)`: `sync_settings()`
  then `retire_leaving_scope(arriving)`, with a doc comment naming the same-batch
  trigger.
- `experience.rs:222,234,247,256,266` — all five switch branches (wetland ->
  sandbox, wetland -> relay, wetland -> terrain lab, lab -> wetland, relay ->
  wetland) now call `begin_scope_switch` before their `take()`.
- `experience.rs:655` — regression test
  `same_batch_preference_change_survives_the_scope_switch_without_about_to_wait`:
  drives the registered SETTINGS zone (`pointer_down == Some(5)` +
  `activate_button(5)`), toggles `SettingRow::LargeControls` and
  `SettingRow::Mute` through `settings_click`, presses DONE, then presses the
  registered MENU zone (`pointer_down == Some(4)` + `activate_button(4)`); with
  no `about_to_wait` it asserts the owner is still stale and the file still
  absent (the pre-switch state), calls the production prelude, and then asserts
  owner equals the toggled settings, the file round-trips them, the adapter is
  muted with scope `Wetland` and zero device failures, and the arriving relay's
  `move_zone` equals `relay_layout(&toggled)` (with an `assert_ne!` proving the
  toggled layout differs from the default, so the check cannot pass vacuously).

## Verification

All commands from the repository root with the dispatch environment
(`RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=../shared-controls-target`).
Scoped to the changed module as dispatched; the full app suite was not repeated.

- **Red step.** With the flush temporarily disabled in `begin_scope_switch`,
  `cargo test -p matterweave-explorer experience::tests::same_batch_preference_change_survives_the_scope_switch_without_about_to_wait`
  exit 1: `assertion left == right failed: the owner must take the queued change
  before the sample leaves; left: SharedSettings { version: 1, handedness: Left,
  large_controls: false, muted: false }; right: SharedSettings { version: 1,
  handedness: Left, large_controls: true, muted: true }` — `0 passed; 1 failed`.
- **Green step.** Same command with the flush restored, then the whole module:
  `cargo test -p matterweave-explorer experience::` exit 0,
  `6 passed; 0 failed; 0 ignored; 206 filtered out`, including
  `experience::tests::same_batch_preference_change_survives_the_scope_switch_without_about_to_wait ... ok`
  and the pre-existing `sample_settings_change_propagates_mute_and_persists`,
  `scope_switch_drops_leaving_events_and_retires_its_voices_first`,
  `pump_plays_active_sample_events_and_suspension_drops_them`,
  `focus_loss_silences_and_focus_gain_cannot_override_lifecycle_suspension`,
  `no_device_before_resume_and_one_adapter_across_lifecycle`.
- `cargo clippy -p matterweave-explorer --all-targets --no-deps -- -D warnings`
  exit 0 (`0 errors`); the only warning is the pre-existing vendored-winit
  `function_casts_as_integer` lint, not app code.
- `cargo fmt --all --check` exit 0 (after one scoped `cargo fmt -p
  matterweave-explorer`).
- `python3 tools/check_docs.py` exit 0, no errors or missing links.
- `git diff --numstat apps/explorer/src/experience.rs` before commit: `107	5`
  (`107` insertions, `5` deletions); one file changed, plus this new log.

Not run (explicitly out of scope for this worker): Android build/APK, phone or
emulator run, physical multitouch, lifecycle restart on device, audible output,
benchmarks, `/mnt/bench`, and the full `matterweave-explorer` suite. Nothing
here is a mobile performance or audio-hardware claim.

## Insights

- The event-batch hazard is not specific to settings: any one-shot state a
  sample writes in `window_event` and the owner consumes in `about_to_wait` can
  be destroyed by a `take()` in the same batch. Making the switch prelude an
  explicit step is what gives that ordering one home instead of five
  branch-local obligations.
- A review finding can be correct and still not directly testable; placing the
  fix at the nearest reachable point on the real path (the prelude immediately
  before retirement) keeps both the behavior and the regression evidence honest.
- The equality guard in `sync_settings` is what makes flushing at switch time
  free: the prelude cannot rewrite an unchanged file, so the fix does not
  reintroduce per-frame writes on the switch path either.
