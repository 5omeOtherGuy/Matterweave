# P02 phone runner: matched 3-pair full-wetland candidate/reference collection

`tools/performance/collect_wetland_pair.py` is the executable supervisor for the
P02 comparison between the two frozen builds. It **collects evidence only**. It
computes no FPS, no energy figure, no verdict, and its output is not proof of a
speedup. Analysis and any comparison belong to the phone lead.

Status of this document: the script is written, syntax-checked and unit-tested
on the host. **No phone step has been executed** (no adb invocation, no install,
no trial). Every device-side claim below is a description of intended behaviour,
not a measurement.

## What is compared

| Variant | Source commit | Difference |
| --- | --- | --- |
| candidate | `0e5b3be` | restores unconditional dynamic geometry rebuild/upload and advisory WSI recreation |
| reference | `3a7f536` | the same tree with only that change removed |

Both frozen APKs live in `performance/completion-02/p02-fullmap/` with adjacent
`*-build.json` manifests holding the exact APK SHA-256, source commit,
generator 2 and scene composition hash `f458591e7b345546`. The supervisor
verifies the host APK hash against its manifest, refuses to start if the two
manifests disagree on composition hash or generator, and after installation
hashes the *installed* file on the device and aborts on any mismatch.

This is a paired optimization experiment, not a proof of a speedup.

## Design invariants

- **Readiness is not re-implemented.** Idle windows are validated by
  `tools/performance/validate_conditions.validate_readiness`: at least 5 samples,
  span >= 120 s, no gap > 35 s, battery spread <= 1 C, skin spread <= 2 C,
  unplugged, non-mock/non-override, thermal status 0. The runner only *calls*
  it; no threshold is duplicated or relaxed anywhere in the runner.
- **Thermal matching.** The first trial's accepted window becomes the matching
  reference for all later trials; every later trial must start within 1 C battery
  and 2 C skin of it (`check_thermal_match`). A previously recorded, too-warm
  reference is never reused.
- **Fail closed.** At most `--max-observations` (default 31) idle observations
  per trial; if no window both passes readiness and matches the reference, the
  trial raises and no capture is launched. Every observation is retained in
  `observations.jsonl`, and every rejection reason in
  `readiness-rejections.json`.
- **Order.** Three pairs, alternating within-pair order AB / BA / AB
  (reference-candidate, candidate-reference, reference-candidate), 120 s warmup
  plus 120 s measurement per trial.
- **Bounded I/O.** Every adb call has an explicit timeout; the only long-lived
  child process is `adb logcat` for the app's own tag, terminated in `finally`.
- **No hidden work on import.** Nothing runs at import time; all device paths
  require explicit CLI arguments.

## User-data safety

The user's world and saves are treated as untouchable:

- `files/world.json` is never read, written, backed up or restored. There is no
  repeated save/restore cycle anywhere in the script.
- The existing `files/wetland-session.json` and the user's existing
  `*.recovery-1.json` experiment are never written. The base save is only read
  to confirm it is invalid for generator 2 - if it is *valid*, the run aborts,
  because then `SavedWetland::load_recovering` would load it and the benchmark
  fixture would not be selected.
- The runner owns exactly one device file:
  `files/wetland-session.json.recovery-127.json`. `load_recovering`
  (`apps/explorer/src/wetland_state.rs`) scans slots 1..=128 and keeps the
  *highest valid* recovery, so slot 127 wins over the user's slot 1 while both
  files stay intact. The first trial requires the slot to be absent; the exact
  provided bytes are written and read back before each trial.
- `detail-gallery.txt` is rejected if present rather than silently overridden,
  and a pre-existing `profile-frames.txt` is rejected rather than consumed.
- Final cleanup removes only the runner's own recovery-127 fixture and, if it
  created one, the profile request. Nothing else is deleted.
- Display settings are never modified. `screen_brightness`,
  `screen_brightness_mode`, `screen_off_timeout` and the refresh-rate settings
  are recorded, and the run aborts if `screen_off_timeout` is below 300000 ms so
  the operator changes it deliberately instead of the script mutating the device.

## Per-trial sequence

1. Install the frozen APK; verify the installed hash on the device.
2. Record device props, settings and starting battery/thermal/display dumps.
3. Record the idle window until readiness and thermal matching pass.
4. Preflight app-private files; write and verify the recovery-127 fixture.
5. Create `files/profile-frames.txt` with `100000`, a bounded capture request.
6. `am start` the `NativeActivity`, screenshot the chooser, tap the entry point
   (`--tap`, default `850 780` on the 3168x1440 device), then wait bounded for
   the app's real `WETLAND LOADED: N cells / M placed objects` log line. If it
   never appears, or the counts are zero, the trial aborts - the runner refuses
   to measure a scene that is not the wetland. A real screenshot is taken after
   the scene settles.
7. Resolve the single `dev.matterweave.explorer/android.app.NativeActivity`
   SurfaceFlinger layer and sample `dumpsys SurfaceFlinger --latency` about
   twice a second across warmup and measurement, storing raw text with raw host
   timestamps. Health (`battery`, `thermalservice`, `meminfo`, `activities`) is
   sampled every 30 s and gates on foreground and unplugged state; a layer with
   no valid presentation history rejects the capture.
8. Whole-process CPU ticks (`/proc/<pid>/stat`) are read at warmup start,
   measurement start and measurement end, with the host window recorded around
   each read. This is an approximate independent window: **no strict
   GPU/compositor/CPU join is claimed.**
9. `KEYCODE_HOME` first, so the app saves and flushes its capture, then
   force-stop. The new `frame-profile-v2-*.csv` is pulled (exactly one required,
   never overwriting an existing file), and the saved session is pulled and
   re-checked for generator 2 and the six frozen bodies.

## Artifacts

Per run (`--out`, must be fresh): `manifest.json` (purpose, adb/serial, both
build manifests plus their hashes, fixture hash, plan, validator and supervisor
SHA-256), `run.log`, `run-complete.json` or `failure.json`, `cleanup.json`.

Per trial subdirectory `pair<N>-<position>-<variant>/`: `trial-request.json`,
`installed-apk.json`, `device-props.json`, `settings.json`, `display.txt`,
`window.txt`, `battery-start.txt`, `thermal-start.txt`, `observations.jsonl`,
`readiness.json`, `readiness-rejections.json`, `files-preflight.json`,
`launch.txt`, `tap.json`, `chooser.png`, `entered.png`, `app.log`,
`scene-loaded.json`, `runtime.json`, `surface-samples.jsonl`, `health.jsonl`,
`process-stat.json`, `frame-profile-v2-*.csv`, `session-after.json`,
`trial.json`, `collection-complete.json`.

Failed trials keep all of the above that they produced; cleanup still runs, and
a partial trial is never presented as a result.

## Running it

```sh
python3 tools/performance/collect_wetland_pair.py \
  --adb /mnt/bench/matterweave-dev/android-sdk/platform-tools/adb \
  --serial 192.168.178.93:42373 \
  --candidate-apk performance/completion-02/p02-fullmap/candidate.apk \
  --reference-apk performance/completion-02/p02-fullmap/reference.apk \
  --fixture <lead-provided wetland session JSON> \
  --out performance/completion-02/p02-pairs-<fresh-name>
```

The fixture must be a version 1, generator 2 session with exactly six frozen
bodies and a finite eye/yaw/pitch/shadows pose; `check_fixture` rejects anything
else before the phone is touched. Expect roughly 6 x (idle window + 240 s) plus
install time; the idle windows dominate and are not bounded by wall-clock
promises here.

## Lead-owned follow-up

- Comparing candidate against reference, and any statistical treatment, is the
  lead's. The runner deliberately produces no summary statistic.
- A scene comparator across trials (saved fixture bytes after each trial plus
  profile row counts) is left as an explicit lead-owned report;
  `session-after.json` and the per-capture row counts in `trial.json` are the
  inputs.
- Frame captures can be validated with
  `tools/performance/validate_frame_profile.py`.

## Verification performed on the host

| Command | Result |
| --- | --- |
| `python3 -m py_compile tools/performance/collect_wetland_pair.py` | pass |
| `python3 tools/performance/collect_wetland_pair.py --help` | pass (no phone needed) |
| `python3 -m unittest test_collect_wetland_pair` (in `tools/performance`) | pass, 12 tests |
| `python3 tools/check_docs.py` | pass |
| any adb / install / trial / measurement | NOT RUN |
