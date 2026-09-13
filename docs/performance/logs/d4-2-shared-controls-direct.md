# Engineering log d4-2-shared-controls-direct

Worker slice: **D4.2 direct shared-service slice** — usable shared mobile settings
for Wetland and Voxel Relay. Branch `engine/shared-mobile-settings`, base
`8183540a631f26bcc1053c363f481e6cd3ec367b` (current `origin/main` after the
fast-forward recorded below). No phone, no APK, no benchmark and no `/mnt/bench`
work was performed.

Owned writes used: new `apps/explorer/src/settings.rs`; `apps/explorer/src/controls.rs`;
`apps/explorer/src/experience.rs`; `apps/explorer/src/wetland.rs`;
`apps/explorer/src/voxel_relay.rs`; `apps/explorer/src/lib.rs` (one module
registration line only); this log. No lighting region was edited: the isolated
wetland-proxy-lighting worker keeps `wetland_lighting.rs`, its `Runtime` lighting
field/install arm and the mesh-lighting diagnostic registration. No
`matterweave-core`/physics/render crate, game-save schema, `Cargo.lock`, CI,
`docs/STATUS.md`, `docs/HANDOFF.md` or performance-board edit was made.

## Actions Taken

1. **Recovered the checkout before writing code.** The dispatch said the prior
   Muse attempt produced no code. `git status --porcelain=v1 -b` reported a clean
   tree on `engine/shared-mobile-settings` at the frozen base `495d9f9`,
   `...origin/main [behind 32]`. `git merge --ff-only origin/main` fast-forwarded
   `495d9f9..8183540` (34 files, +4812/-85) with no conflicts and no preserved
   local changes to report.
2. **Read the contracts first**: `AGENTS.md`, `docs/REQUIREMENTS.md` (R10
   adjustable layout), `docs/ENGINE_COMPLETION_PLAN.md` (D4/D4.1/D4.2),
   `controls.rs`, `InputService` (`crates/matterweave-core/src/input.rs`),
   `experience.rs` + `audio_service.rs` (`AudioAdapter::set_muted`, status), the
   wetland chooser/in-game HUD and Voxel Relay HUD/`setup_input`/touch paths.
3. **New `settings.rs`**: `SharedSettings { version, handedness, large_controls,
   muted }`, `Handedness`, `SettingRow`, strict `read`, never-failing `load`,
   atomic `save` (sibling `.tmp` + `sync_all` + `rename`), `MAX_SETTINGS_BYTES =
   4096`, `SETTINGS_VERSION = 1`, `serde(deny_unknown_fields)`. File name
   `matterweave-settings.json` in the application data directory; it is a sibling
   of `world.json` and never a game save.
4. **One layout calculation per surface in `controls.rs`**: `settings_panel()`
   (panel rect, three rows, DONE), `settings_panel_click`, `draw_settings_panel`,
   `wetland_layout`, `relay_layout`, `action_at`, `virtual_point`. Default
   rectangles reproduce the shipped wetland/Relay layouts exactly; handedness
   mirrors the primary zones; `large_controls` scales the primary zones.
5. **Wetland integration** (`wetland.rs`): chooser SETTINGS button, in-game
   options `SETTINGS` row, modal overlay, layout used by both `hud()` and
   `click()`/`start_wetland`, one-shot change queue, modal keyboard guard.
6. **Voxel Relay integration** (`voxel_relay.rs`): registered `SETTINGS` button
   zone in the top row, modal overlay, `setup_input` rebuilt from `relay_layout`
   (now replacing zones instead of appending), `activate_button`, one-shot change
   queue, modal touch/keyboard routing.
7. **Owner policy in `experience.rs`**: `Experience` loads the settings file
   once, installs the same `SharedSettings` into each sample it constructs,
   persists a sample's change in `sync_settings` (one write per real change),
   calls `AudioAdapter::set_muted` and re-asserts mute after every resume. The
   adapter stays the one device owner; mute is not a gameplay event and no second
   adapter exists.
8. **Tests and checks**: 28 new focused tests (7 settings, 8 controls, 7 wetland,
   5 Relay, 1 Experience owner test) plus scoped strict Clippy/fmt, then this log.

## Issues & Friction

- **Prior attempt produced no code.** The checkout was clean at the frozen base;
  after the fast-forward there was nothing to preserve. The FF was possible
  because the branch had no unique commits.
- **Mirrored Relay ACTION coincides with the default stick area.** The first
  held-touch regression asserted that the old default-stick centre was inert
  after swapping to right-handed. In the mirrored layout that point is the
  right-hand ACTION button (`[40, 450, 200, 70]`), so a *new* press there
  legitimately registers an action. The property that matters — a stale or
  replayed touch at the old stick position produces no movement — was kept and
  the assertion corrected; the live mirrored stick and ACTION are now checked
  separately.
- **Relay `setup_input` appended button zones.** `InputService::clear()` clears
  pointers/keys/deltas, not zone configuration, so every `reset()` appended four
  more identical button zones. Harmless to first-match hit-testing but unbounded.
  The layout rebuild now clears button/motion zones first; this is required for
  layout changes anyway.
- **Open modal still moved the player via keyboard.** Touch was already routed to
  the overlay, but desktop keys kept feeding movement. Both samples now ignore
  gameplay keys while the overlay is open; relay Escape closes the overlay
  instead of returning to the chooser (wetland Escape already closed it).
- **Relay move zone cannot be fully unset while the overlay is open.**
  `InputService` exposes `set_move_zone` but no clear, so the previous rect stays
  configured although `setup_input` registers no zones and every touch is routed
  to the overlay. No `matterweave-core` change was made for this; it is a
  documented, unreachable path.

## Decisions & Rationale

- **Preferences are data, layout is geometry.** `settings.rs` holds values,
  validation and persistence; `controls.rs` turns values into rectangles. This
  keeps the panel rows and the drawn/hit regions derived from one calculation
  (`settings_panel()`), satisfying "visual rectangles and hit regions use the
  same layout calculation" literally for both samples.
- **One writer: `Experience`.** Samples own a copy for rendering/editing and
  expose `set_shared_settings` / `take_settings_change`. The owner persists and
  is the only code that touches `AudioAdapter::set_muted`. This matches the D4.1
  ownership split ("one owner applies audio policy") and keeps `audio_service.rs`
  untouched.
- **One-shot change queue instead of callbacks.** Single-threaded frame loop:
  `sync_settings()` runs once per `about_to_wait`, compares against the owner's
  copy and writes only on a real difference. An open chooser therefore never
  rewrites the file per frame.
- **Atomic bounded file.** Temp file in the same directory, `sync_all`, then
  rename: a crash leaves the old or the new document. A 4 KiB bound, compiled-in
  version and `deny_unknown_fields` mean corrupt, truncated, oversized or
  future-version documents fall back to defaults at startup and are never
  partially honored. The loader never creates the file and never opens a save.
- **Mirroring rules.** Wetland mirrors stick, jump and the five action buttons
  together (right-handed play moves the whole cluster). Relay mirrors only the
  stick and ACTION, with ACTION always opposite the stick; mirroring the top row
  would collide with the 610-px status panel. Defaults are the shipped
  rectangles, locked by tests.
- **Large layout values are explicit, not a scale factor.** Static values keep
  every rectangle integral, on-canvas and non-overlapping for all four
  combinations (verified by test), and keep the HUD text anchors derived from the
  rectangles so labels follow the mirrored/scaled controls.
- **Sandbox (`Explorer`) is intentionally not wired.** It already persists
  `swapped`/`large` inside its own session save; pushing shared values into it
  after construction would clobber that saved layout. The slice targets Wetland
  and Voxel Relay, so `lib.rs` received only `mod settings;` and no sandbox
  setter. If the lead wants sandbox parity later, it needs an explicit
  precedence decision versus the existing session fields.
- **Modal overlay clears contacts on open, toggle and close.** `controls.clear()`
  / `setup_input()` drop every pointer, so a held thumb cannot keep steering
  across a layout change or "click through" the overlay.

## Solutions Applied

- `settings.rs`: versioned bounded document with `load`/`read`/`save`, value
  labels, per-row toggle; 7 tests cover round-trip, atomic replace, missing file,
  malformed/unknown-field/unsupported-version/invalid-value/oversized input,
  label mapping and the encoded-size bound. Save/load tests also assert that two
  sentinel game-save files are byte-identical afterwards.
- `controls.rs`: `WetlandLayout`/`RelayLayout`/`SettingsPanel` plus shared
  `virtual_point`. Tests lock the shipped defaults, prove all four handedness ×
  size combinations stay inside 1000×600 with pairwise-disjoint zones, prove
  mirroring, panel hit-testing, narrow/wide viewport round-trips, and that
  `start_wetland` uses the provided layout and drops stale contacts.
- `wetland.rs`: `settings`, `settings_dirty`, `settings_open` state; panel route
  from chooser and in-game options; HUD and clicks both call `wetland_layout` /
  `action_at`; `set_shared_settings`/`take_settings_change`; Escape closes the
  overlay.
- `voxel_relay.rs`: `settings`/`settings_dirty`/`settings_open`; `setup_input`
  replaces zones and registers nothing while modal; action 5 opens the overlay;
  `settings_click` routes touches; HUD uses `relay_layout` (stick, ACTION, top
  row and the new SETTINGS button, labels included); `activate_button` is the one
  action dispatcher.
- `experience.rs`: settings path + value loaded in `new`, installed into the
  wetland and every later relay/wetland construction, `sync_settings` persists +
  mutes, `resumed` re-asserts mute.
- `lib.rs`: `mod settings;` only.

## Verification

All commands from the repository root with the dispatch environment
(`RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/shared-controls-target`).

- Recovery: `git status --porcelain=v1 -b` → clean, behind 32; `git merge
  --ff-only origin/main` → `Updating 495d9f9..8183540 Fast-forward`, exit 0.
- `cargo check -p matterweave-explorer --all-targets` → `0 errors`; the only
  warning is the pre-existing vendored winit X11 function-cast lint.
- `cargo test -p matterweave-explorer` → exit 0; main suite `210 passed; 0
  failed; 1 ignored; finished in 6.21s`. The ignored test is the explicit opt-in
  `wetland::integration_tests::full_wetland_load_edit_collision_and_reload`
  (full-map load/generate), unchanged by this slice.
- `cargo test -p matterweave-explorer -- --list` → 28 new named tests present:
  `settings::tests::{defaults_match_the_existing_sample_layout_and_audio,
  round_trip_preserves_values_and_touches_no_game_save,
  save_replaces_previous_values_atomically,
  missing_file_loads_defaults_without_creating_anything,
  invalid_files_fall_back_to_defaults_and_remain_replaceable,
  toggle_turns_each_row_and_value_labels_follow,
  encoded_document_stays_inside_the_bound}`;
  `controls::tests::{wetland_default_layout_matches_the_shipped_rectangles,
  relay_default_layout_matches_the_shipped_rectangles,
  every_layout_combination_fits_the_canvas_without_overlapping_zones,
  handedness_mirrors_the_primary_zones_around_the_canvas_center,
  settings_panel_geometry_is_disjoint_and_hit_testing_toggles,
  settings_panel_draws_for_every_value_without_panicking,
  virtual_point_maps_narrow_and_wide_viewports_into_the_drawn_rectangles,
  wetland_start_uses_the_shared_layout_and_swaps_cleanly}`;
  `wetland::shared_settings_ui_tests::{chooser_settings_button_opens_the_shared_panel,
  panel_toggles_queue_one_shot_changes_and_done_closes,
  in_game_options_route_reaches_the_same_panel,
  action_hit_regions_follow_handedness_and_scale_with_the_shared_layout,
  applying_a_layout_change_clears_held_touches,
  owner_install_updates_the_rendered_preferences_without_queueing_a_change,
  narrow_and_wide_viewports_map_touches_into_the_drawn_rectangles}`;
  `voxel_relay::shared_settings_ui_tests::{settings_overlay_opens_from_the_registered_hud_zone,
  overlay_toggles_queue_one_shot_changes_and_apply_the_layout,
  installed_zones_match_the_shared_layout_for_every_preference,
  held_touch_cannot_steer_a_swapped_layout, preferences_never_touch_the_relay_save}`;
  `experience::tests::sample_settings_change_propagates_mute_and_persists`.
- `cargo clippy -p matterweave-explorer --all-targets --no-deps -- -D warnings` →
  exit 0; only the vendored winit lint remains (not app code).
- `cargo fmt --all --check` → clean after `cargo fmt --all`.
- `python3 tools/check_docs.py` → exit 0, no errors or missing links after this
  log was added.

Not run (explicitly out of scope for this worker): Android build/APK, phone or
emulator run, physical simultaneous multitouch, lock/unlock, audible-output
confirmation, benchmarks and `/mnt/bench`. Nothing here is a mobile performance
or audibility claim. D4/D4.2 completion is not claimed.

## Insights

- Because every sample renders the HUD into the same 1000×600 space and maps
  touch through the same affine projection, viewport aspect changes scale the
  whole control surface uniformly; the testable invariant is therefore that
  draw/hit rectangles are one calculation and stay inside the canvas for every
  preference combination, not a per-aspect branch.
- The cheapest way to make "layout change clears held touches" true is for the
  rebuild path itself to be the clear path (relay `setup_input`), rather than
  remembering to clear at each call site.
- Two independent input paths (wetland immediate `click` actions vs relay
  `InputService` button zones) both become preference-driven by sharing a layout
  type, without unifying their dispatch behaviour or touching `InputService`.

## Proposed shared-status updates (device/delivery worker owns those files)

- `docs/STATUS.md`: add a D4.2 entry: shared bounded settings
  (`matterweave-settings.json`, v1, 4 KiB) with handedness / normal-large /
  mute, reachable from the wetland chooser, wetland in-game options and Voxel
  Relay HUD; owner persists and applies mute; 28 focused host tests pass;
  Android/device and physical multitouch gates NOT run.
- `docs/HANDOFF.md`: next actions — lead review of the frozen head commit, then
  combined Android acceptance for settings reachability, restart persistence,
  mute lifecycle and physical multitouch; lighting worker merge must preserve
  `SharedSettings` fields added to `wetland.rs` (`settings`, `settings_dirty`,
  `settings_open`) and the `mod settings;` line in `lib.rs`.
- `docs/performance/board.json`: no performance row; this slice makes no timing
  claim. If a row is required, mark it functional with the host test counts above
  and both device gates `NOT RUN`.
