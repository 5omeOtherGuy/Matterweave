# Engineering log: shared mobile settings Android delivery (PR #53)

Delegated integration/acceptance worker (`deepseek-settings-device`,
Category: test) · 2026-09-13 · OnePlus 13 `192.168.178.93:5555` (CPH2653,
Android 16, Adreno 830, Vulkan 1.3.284, 3168x1440).

Reviewed subject: `engine/shared-mobile-settings` head
`4b4fbcd2f286719fdf0966a25de811ac146d1f4b`, accepted by Opus review
`w_5b4c3413` (`opus-settings-review`) and corrective review `w_69103511`
(`opus-settings-ordering-review`, ACCEPT at the frozen head, no blockers).
Authored by `w_6f7430c8` (base slice) and `w_a57a5cb8` (same-batch ordering
repair). PR [#53](https://github.com/5omeOtherGuy/Matterweave/pull/53).

Scope: integrate the frozen head onto current main in an isolated acceptance
checkout, build **one** APK, exercise the shared settings in the chooser, the
Wetland options and the Relay HUD on the phone, restore the owner's five saves
and the prior settings-file absence exactly, then merge PR #53 with normal
required CI. No lighting source was integrated (PR #54 stayed untouched), no
`/mnt/bench`, no inherited `sccache`.

## Actions Taken

1. Reconciled git before touching anything: `origin/main` was `54ec529`
   (PR #48/#49/#51/#52 merged, matching the handoff); `refs/pull/53/head` was
   still `4b4fbcd`, and `refs/pull/53/merge` was `6b341172` (PR #53 x
   `54ec529`, tree `4797b78c`). PR #53's own required checks were re-run and all
   six (docs, host, android x2) passed before merge.
2. Built the acceptance artifact from GitHub's own merge commit
   `6b341172` in an isolated worktree `settings-acceptance` (detached), so the
   device evidence is tied to the exact merge tree rather than a hand-made
   merge.
3. Built exactly one APK with the prescribed healthy environment
   (`ANDROID_HOME`, `GRADLE_USER_HOME`, warm `detail-android-target`,
   `CARGO_BUILD_JOBS=1`, empty `RUSTC_WRAPPER`). `verify_apk.py` PASS (ARM64,
   16 KiB LOAD alignment), `apksigner verify` v2 debug certificate,
   `adb install -r` Success, on-device `sha256sum` equal to the local APK.
4. Captured the phone pre-state before any test: no
   `matterweave-settings.json` (prior preference state was **absence**), the
   five current saves byte-equal to the earlier handoff backup
   (`f0f37bcc…`, `e474b8c9…`, `44f102b1…`, `4f66fd00…`, `d53380cf…`), app
   force-stopped, `bff9e869…` (detail-48) still installed. Backed the whole
   files directory up privately to
   `orchestration/deepseek-settings-device/device-backup/` (0700) with a
   SHA-256 manifest.
5. Ran staged on-device phases with screenshots, per-phase logcat excerpts and
   settings-file snapshots: chooser panel, Wetland in-game options, Relay HUD,
   handedness x size combinations, the mute round trip, the fast
   DONE-then-switch path, HOME/resume, cross-sample switching, restart, and a
   final cleanup that restored saves and removed the test-created settings
   file.
6. After the owner's mid-run steering merged PR #55 (`d37ca73`, docs-only),
   recomputed the merge of the frozen head onto the new main with
   `git merge-tree`: tree `14ad21ff…`, which differs from the built tree
   `4797b78c…` **only** by PR #55's four documentation files (zero code
   differences). No rebuild was needed and the device evidence transfers
   exactly; the earlier phases were not replayed against a redundant binary.
7. Merged PR #53 as a normal merge commit `f878e1b` on `d37ca73`; the merged
   tree is exactly `14ad21ff…`, so GitHub's merge result matches the predicted
   and device-tested code tree.

## Delivery gates

| Criterion | Verification method | Result |
| --- | --- | --- |
| Android settings | Both samples, three entry points, layout + input + mute + persistence + HOME | **PASS** (chooser, Wetland options, Relay HUD; left/right x normal/large; file-backed mute; restart and sample switch; HOME/resume input) |
| Save integrity | Fresh 5 saves + prior preference state restored exact, corrected APK installed | **PASS** (5/5 byte-equal to the fresh backup; settings file removed, prior state was absent; `e6f4c4b8…` installed) |
| PR #53 delivery | Independent review at the frozen SHA, required CI, device pass, merged | **PASS** (Opus `w_5b4c3413` + corrective `w_69103511` accept; 6/6 checks green; merged `f878e1b`) |
| Handoff | Shared status/board/log exact, physical multitouch still open | **PASS** (STATUS/HANDOFF/board updated; physical simultaneous multitouch recorded OPEN) |

## Device evidence (observed, not inferred)

- **Chooser → SETTINGS** at virtual (930,41): panel drew with
  `MOVE STICK LEFT / CONTROL SIZE NORMAL / SOUND ON`; toggling the rows chased
  `RIGHT`/`LARGE`; **no** settings file existed while the panel was open, and
  DONE wrote `matterweave-settings.json` for the first time
  (`{"version": 1, "handedness": "right", "large_controls": true,
  "muted": false}`, 87 bytes, `ba4a341f…`). The post-DONE chooser frame was
  byte-identical to the pre-settings chooser frame, so the overlay left no
  ghost state.
- **Wetland (right + large)**: MOVE/JUMP drew on the right and the five actions
  on the left at the large rectangles. Holding the **drawn** stick centre moved
  the diagnostics position `85.0 19.5 53.6` → `86.4 19.5 51.0`; holding the
  default left-hand stick position produced **no** position change (old zone
  inert). PLACE/REMOVE at the drawn rectangles logged
  `WETLAND EDIT 17.509ms Voxel added` and `13.763ms Voxel removed` with
  `Voxel added/removed` status text visible; a mid-screen drag rotated the view
  and did not disturb the position.
- **Wetland options entry**: MENU → `SETTINGS` (fifth options row) opened the
  same panel; the three rows are the same rectangles as in the chooser.
- **Mute policy**: SOUND→OFF wrote `muted: true` (86 bytes, `0dc9175c…`) and a
  subsequent PLACE logged
  `Sample audio Wetland: event=block-edit outcome=Dropped(Muted)`; SOUND→ON
  wrote `muted: false` and the next PLACE logged `outcome=Started`. This is
  on-device adapter state from the real audio path, not a host counter; no
  human-hearing claim is made (the owner's baseline audition stands).
- **Fast DONE-then-switch**: handedness toggled and DONE followed by
  `RETURN TO MENU` within about a second still persisted `handedness: left`
  (`1248c0ff…`) and re-derived the arriving layout: on re-entry only the
  left-hand large PLACE rectangle produced an edit, and the abandoned mirrored
  rectangle produced none.
- **HOME/resume**: a real HOME (`keyevent 3`) plus `am start` left the settings
  bytes identical and kept gameplay input alive — the drawn stick moved
  `89.1 18.3 60.7` → `89.7 18.3 60.8` after resume. The adapter logged its
  usual resume-time `stream start failed (code -899)` degradation and recovered;
  that is pre-existing device behavior, unrelated to settings.
- **Relay (HUD + mirrored zones)**: the HUD `SETTINGS` button opened the
  panel; toggling CONTROL SIZE persisted and re-derived the layout. The
  **drawn** mirrored stick drove the character (physics eye read back from the
  relay save attachment (x/z): `10.0/1.6` → `4.67/9.68`), the crate push opened the
  door (`door_open: true`, `objective` event), and the **drawn** mirrored ACTION
  button cleared the obstacle (`obstacle_cleared: true`, status
  `OBSTACLE CLEARED`, `Sample audio VoxelRelay: event=block-edit
  outcome=Started`). Holding the old left-hand stick position left the physics
  eye bit-identical over 1.5 s.
- **Cross-sample persistence**: a preference changed in the Relay panel
  (left/normal) propagated into a freshly constructed Wetland (left/normal,
  PLACE at the left rectangle logged `Voxel added`), and a later restart showed
  the file unchanged (`825f44f4…`) with the persisted right/normal layout and a
  working mirrored PLACE zone.
- **Cleanup**: all five saves re-hashed on device byte-equal to the fresh
  backup, the test-created `matterweave-settings.json` removed (prior state was
  absence), no `.tmp` left behind, no new files in `files/`, app force-stopped,
  and `e6f4c4b8…` still installed.

Artifact identity: APK
`e6f4c4b8d11ae37f1ff2c0d13fa1261e7741ef5ad26ff25c7eaab7bb04fa4f3f`, built from
tree `4797b78c4cf08236d2b7685d26dc0d4803d10a5c` (commit `6b341172` =
GitHub's `refs/pull/53/merge`), merged as `f878e1b` with tree `14ad21ff…`
(code paths byte-identical to the built tree; only PR #55's docs differ).
Private evidence: `orchestration/deepseek-settings-device/device/`
(screenshots, per-phase logs, logcat excerpts, settings snapshots,
`prestate.json`, `cleanup.json`) and `device-backup/`.

## Issues & Friction

- **First mute attempt was not discriminating.** The first muted PLACE tap was
  refused by the game before it produced an event
  (`WETLAND EDIT 10.092ms Step back before placing here`), so no `Sample audio`
  line existed to prove the mute. Instead of reporting the mute as "verified"
  from the file alone, the phase was repeated after stepping back from the
  placed voxel, which produced the three-line
  `Started → Dropped(Muted) → Started` discrimination.
- **The Relay ACTION is conditional**, so a button press that changes nothing
  is not evidence of a broken hit region. The first presses returned false
  (`obstacle_cleared` stayed false, no event); only after steering the character
  into the corridor (x 4.67 → 6.28, z 9.68 → 14.48, read from the physics eye)
  did the drawn ACTION rectangle clear the obstacle. The three out-of-range
  presses are recorded as no-ops, not as passes or failures.
- **Relay steering is coarse with injected sticks.** The first push overshot the
  doorway (x 4.99 is outside the 5..7 opening); small stick offsets plus
  save-readback were needed to align. This cost several phase runs before the
  owner's steer capped the maze work at one meaningful ACTION.
- **The "same batch" flush race is not reproducible with ADB taps.** Each
  `input tap` is its own event batch, so the device run only exercises the
  rapid sequential path; the same-batch `DONE + switch` case is proven by the
  worker's `same_batch_preference_change_survives_the_scope_switch_without_about_to_wait`
  regression test (red/green per the corrective review), not by the phone.
- **Cosmetic observation, not repaired**: the Wetland load-time hint still
  reads "Left thumb moves. Drag right to look." in right-handed mode. It is
  load-time status text, not a control rectangle, so the touch-region invariant
  is unaffected; it is recorded here instead of being self-reviewed and
  repaired.
- **PR #55 landed mid-run.** The acceptance base moved from `54ec529` to
  `d37ca73` while the device gate was running. Rather than rebuild by reflex,
  the predicted merge tree was computed and compared: docs-only delta, so the
  single built APK stayed the delivery artifact.

## Decisions & Rationale

- Built from GitHub's `refs/pull/53/merge` commit instead of merging locally:
  the device evidence then belongs to the same tree GitHub would create, and the
  final merge could be verified against the prediction
  (`git rev-parse origin/main^{tree}` == `14ad21ff…`).
- Kept the previously-installed `bff9e869…` until the acceptance APK was ready,
  and installed with `-r` only (never uninstall/data-clear), preserving all
  owner data.
- Verified pre-state rather than trusting the dispatch: the brief named APK
  `45ab5de`, but the phone actually held `bff9e869…`; the actual device state
  and the fresh hashes were used. Likewise the dispatch's "current APK" and
  "fresh saves" were re-checked before any write.
- Used the diagnostics position and the relay save's physics eye as ground
  truth, so "the control responds where it is drawn" is a measured fact rather
  than a screenshot impression; the inert-zone checks use the same measurement.
- Restored the prior **absence** of the settings file instead of leaving a
  test-created preference file; the owner never had shared preferences on this
  build.
- Recorded acceptance verdicts in the durable repo docs: the local router CLI
  (`pi-orch.mjs`) exposes run/steer/status but no verdict command, and raw
  worker result files are immutable, so worker records were left untouched.

## Solutions Applied

- Acceptance worktree `settings-acceptance` (detached at the GitHub merge
  commit) with the prescribed healthy build environment; one APK built and
  installed after `verify_apk.py` and `apksigner` verification.
- Staged device harness
  `orchestration/deepseek-settings-device/run_settings_gate.py` (phases
  `prestate`, `chooser`, `wetland`, `mute`/`mute2`, `home`, `fastswitch`,
  `relay`/`relay2`/`relay3`/`relay4`/`relay5`/`relay6`, `restart`, `cleanup`)
  with screenshots, logcat excerpts, settings snapshots and a restore phase.
- PR #53 merged as a normal merge commit after the frozen head's reviews were
  re-read and all six required checks passed on the current head.
- Shared status updated in this delivery: `docs/STATUS.md`,
  `docs/HANDOFF.md`, `docs/performance/board.json`; the D4.3 authoring delivery
  text proposed by `authoring55-delivery.md` was applied with PR #55's merge
  SHA and the now-stale "#53 unmerged" clause corrected to the merged fact.

## Insights

- The strongest settings evidence turned out to be the three-way
  `Started → Dropped(Muted) → Started` sequence: it ties the visible toggle to
  the settings file *and* to the real adapter decision point, and it would be
  impossible if the mute were only cosmetic.
- "Displayed controls are the touch zones" is cheap to falsify on device once
  the old layout's position is held inert: the mirrored-large Wetland stick and
  the mirrored Relay stick both moved the world from the drawn rectangle while
  the abandoned rectangle moved nothing.
- A refactor that clears button zones (PR #53's relay `setup_input`) deserves a
  device check in both handedness states, because a stale-zone bug would look
  exactly like a working layout in a screenshot.
- The `git merge-tree` prediction kept a mid-run base change cheap: an accepted
  docs-only PR does not invalidate a built artifact, and proving the code-tree
  equality is stronger and faster than rebuilding to "be safe".

## Verification (exact commands)

- `python3 tools/verify_apk.py android/app/build/outputs/apk/debug/app-debug.apk`
  → PASS; `android-sdk/build-tools/35.0.0/apksigner verify --print-certs` →
  v2 debug certificate.
- `adb -s 192.168.178.93:5555 install -r …/app-debug.apk` → Success;
  `sha256sum $(pm path dev.matterweave.explorer …)` →
  `e6f4c4b8d11ae37f1ff2c0d13fa1261e7741ef5ad26ff25c7eaab7bb04fa4f3f`.
- `git merge-tree --write-tree d37ca73 4b4fbcd` → `14ad21ff…`;
  `git rev-parse origin/main^{tree}` after merge → `14ad21ff…`;
  `git diff --stat 4797b78c 14ad21ff -- apps crates android Cargo.toml Cargo.lock`
  → empty.
- Device phases as recorded in
  `orchestration/deepseek-settings-device/device/*.log` and
  `device/logcat/*.txt`.
- Save restore: on-device `sha256sum` of the five saves equals
  `device-backup-manifest.sha256`; `matterweave-settings.json` absent;
  `ps -A` shows no explorer process.

## Unrun gates and limits

- **Physical simultaneous multitouch remains OPEN** for both samples, here and
  in the authoring acceptance: sequential ADB injection cannot claim
  simultaneous physical contacts. Lock/unlock remains unrun for the same
  reason. This delivery does not close them.
- The same-batch `DONE + scope switch` flush is proven by the unit regression,
  not by device taps (see Issues); the device run only exercised the rapid
  sequential path.
- No performance, thermal, memory or sustained-workload measurement; the
  diagnostic FPS field is the app's pacing display, not a measurement. No
  frame-time budget is claimed.
- No human-hearing claim: mute is verified by adapter state and logs; the
  owner's earlier audition remains the audibility evidence.
- Wetland load-hint copy is not layout-aware (cosmetic, recorded above, not
  repaired).
- PR #54 (proxy GI/reflection) was deliberately **not** integrated: its
  corrective review landed after this batch's build and the owner scheduled it
  as the next device-functional batch.
