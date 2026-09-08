# Native route replay source review

Frozen `c0c73e0`, integrated as `4e899b3`.

No findings.

Checked `wetland_replay.rs` (full) + `wetland.rs` replay integration, plus `FIXED_DT`/`grounded`/`teleport`/replay call-sites. The five request/stall/completion tests passing is consistent with what I read; real phone replay remains unrun and per instructions is not treated as a code bug.

Why no finding:

* Actual route validation: ordinary `physics.step(dt, replay.velocity(...))` through the normal render loop; no teleport/jump/quality change. Jump explicitly suppressed during replay (`wetland.rs: ~step(dt, velocity, !replay_active && motion.y > 0)`); `Home` teleport path is preceded by `cancel_replay` on `KeyboardInput`. Fall/respawn/off-route/unsettled/waypoint and total-wall timeouts all terminal `FAIL`; completion requires grounded + ≤0.2 m horizontal + ≤1 m eye-height per waypoint.
* Cancellation: `KeyboardInput`/`Touch`/`MouseInput`, `Focused(false)`/`CloseRequested`, mouse-look `CursorMoved`, plus `suspended`/`exiting` all call `cancel_replay`; `finish` is idempotent via `active()` guard and post-terminal `observe`/`velocity` early-return, so no route-index out-of-bounds after `PASS`/`FAIL`/`CANCEL`.
* Default behavior: missing `wetland-replay.json` returns `Ok(None)`; invalid file is rejected and preserved; request consumed once with atomic `create_new` + temp-rename result report. No render/lighting/physics default altered.
* Report accuracy: `RUNNING/PASS/FAIL/CANCEL` + reason, `next_waypoint`/`max_index`, actual/start eye, wall vs accumulated `dt`, physics steps, unix start/end; no simulated duration. Initial + 5 s progress + terminal `persist` with `sync_all`; persist errors logged, not panicked.
* Panic/lifetime: empty-route and `next_waypoint == len` paths guarded by early `FAIL`/`PASS` + `active()` returns before `route[0]`/`route[next]` indexing; `normalize_or_zero` and `(dt+FIXED_DT).max(FIXED_DT)` avoid div-zero; `path.parent().unwrap()` safe for `directory.join(filename)`.

Engineering log:

* Actions: read `wetland_replay.rs`, `wetland.rs`; grepped replay/`FIXED_DT`/`grounded`/`teleport` call-sites.
* Issues: none substantiated.
* Decisions: no code change (read-only leaf).
* Solutions: N/A.
* Insights: double `observe` per frame (pre-step `dt=0` + post-step `frame_dt`) is intentional progress/failure sampling, not double-counting; single `replay_checked` means a late-arriving request file after first in-game frame is ignored by design for this opt-in diagnostic.

Checks not run: real phone replay acceptance; `cargo test`/build; device lifecycle suspend/resume; capture/output inspection; `tools/check_docs.py`.

Lead integration check: native Xvfb smoke exposed a headless focus startup gap.
`73c9b87` adds the actual-app regression; `2b08194` admits replay in explicitly
bounded headless smoke runs using the same allowance as rendering. Corrected
native run advances the route and records CANCEL on exit;25frames/20valid rows,
six bodies and no Vulkan errors. Phone replay remains unrun.
