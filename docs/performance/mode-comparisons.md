# Stationary mode comparisons (gen3)

`tools/performance/collect_wetland_modes.py` collects two **separate** experiments
on one frozen APK. The lead owns exclusive phone access, visual review and
interpretation. Do not run while P02 or another collector owns the phone.

| Experiment | A | B | Held fixed |
| --- | --- | --- | --- |
| `profiling` | profiling on | profiling off | explicit60Hz target |
| `frame-cap` | explicit60Hz | explicit30Hz | profiling on |

Default order is AB/BA/AB (three pairs); `--pairs 1` is exploratory. Never combine
the factors into one overhead/optimization verdict. Targets are requests, not
proof of achieved presentation cadence. Default application behavior remains60Hz.

## Inputs and execution

Use a frozen APK containing the agreed `SavedWetland.frame_rate: u32` field
(30 or60, serde default60). The collector explicitly stages that field and
requires it to be saved explicitly after each trial. Older APKs are unsuitable.
The fixture must be the lead's known-valid six-body, empty-edit gen3 session;
it is copied locally and only `frame_rate` changes. Camera position must remain
within0.01m of the fixture and paired endpoint; yaw/pitch tolerance is1e-6 and
shadows must match. App body restoration is not reimplemented in Python.

`--build` names `build-manifest.json`; its `apk` is a filename alongside it.
Required fields are `apk`, `apk_sha256`, full40-hex `source_commit`, and `scene`
with `generator: 3`, `seed: 20260908`,
`composition_hash: "dfb9f40519a3c151"`. Optional `expected_scene` must equal
`{"cells": 34716467, "instances": 8302}`. Actual load counts are always checked.
The local hash is checked before device access; the installed hash is checked
on **every** trial. Use the same frozen build for both independent runs.

Lead-only execution, after exclusive ownership is established:

```sh
python3 tools/performance/collect_wetland_modes.py \
  --adb /absolute/path/to/adb --serial DEVICE \
  --build /absolute/frozen/build-manifest.json \
  --fixture /absolute/six-body-gen3-session.json \
  --out /absolute/fresh-profiling --experiment profiling

python3 tools/performance/collect_wetland_modes.py \
  --adb /absolute/path/to/adb --serial DEVICE \
  --build /absolute/frozen/build-manifest.json \
  --fixture /absolute/six-body-gen3-session.json \
  --out /absolute/fresh-frame-cap --experiment frame-cap
```

Both default to120s warmup and120s measurement (`--warmup`, `--measure`).
Normal chooser entry reuses the existing850,780 tap and launch/readiness helpers;
the lead must verify this geometry for the target phone. Settings and display
are recorded, not modified. Settings, device properties and display size/density
must match across each pair and across each trial's start/end. Full display dumps
remain raw evidence; active refresh selection is not asserted equal when the
frame target changes.

## Safety and matching

- Base `wetland-session.json` must already be provably invalid for gen3. Missing
  or not-provably-invalid base rejects the run. Never overwrite it to enable a test.
- Reject pre-existing `profile-frames.txt`, `wetland-replay.json`,
  `detail-gallery.txt`, recovery127 or higher recovery128. Only this run's
  already-owned recovery127 may be reused. Even an owned stale profile request
  rejects the next trial. Lower/invalid journals remain untouched.
- Own only recovery127 and, for profile-on, the240000-frame request. Request bytes
  and staged fixture bytes are read back. New controls/competing recovery detected
  before launch or after capture reject the trial.
- HOME flushes the save/CSV; force-stop precedes pulls and cleanup. Saved mode,
  pose and six-body/empty-edit identity are checked. Observed inode/mtime must
  change after staging: unchanged fixture bytes alone cannot prove it was loaded.
- Profile-off requires no new frame-profile output. Profile-on requires exactly
  one validator-passing CSV and a consumed request. Exhausting240000 rows rejects
  the trial rather than silently measuring a partly disabled capture.
- Each pair starts a **fresh**, actually observed first-member readiness window.
  The second member matches that member's actual pre-launch reading, not a shared
  earlier baseline. Battery delta ≤1C, skin delta ≤2C; no±2C battery loophole.
  Reuse the authoritative120s readiness validator, maximum61 observations at30.05s,
  and its fresh pre-launch gate. No synthetic timestamps or relaxed gates.
- Cleanup requires a successful force-stop, then uses the existing owned-file
  cleanup. Failed stop preserves even owned files and records the blocker. No
  save backup/restore. CSVs and other unowned outputs remain on device; all pulled
  raw evidence remains local. A failed trial may leave its CSV only on device.

## Evidence and limits

Per-trial directories retain the mode label, build/source/hash, staged fixture,
settings/display dumps, readiness observations, actual gate, entry screenshots,
app log, compositor/process/health capture, saved session and any new CSV.
`collection-complete.json` is emitted only after trial checks, including pair
matching for the second member. First-member completion alone is not a pair.

`raw-summary.json` reuses the offline supported/co-observed SF interval and
process-CPU helpers. Unsupported history gaps and selected-span coverage remain
explicit; selected-span coverage is not requested-window coverage. App statistics
cover the whole CSV including startup/warmup, with **no exact app/SF clock join**.
Profile-off app/GPU timing metrics remain null/missing, never zero or inferred.
Raw health data remains available in both modes. Summary failures retain a missing
reason rather than inventing intervals. No energy, overhead acceptance,
optimization, significance or sustained-performance verdict is emitted.

## Engineering log — 2026-09-08

- Bounded worker changed only the new collector, its test file and this document.
  Existing P02 collector/analyzer and app code were not edited. Reused Python
  stdlib JSON/hash/path/process APIs and existing readiness/capture/analysis
  helpers; custom logic is confined to the gen3 and independent-mode contracts.
- RED: new tests failed because `collect_wetland_modes` did not exist; checkpoint
  `5a8f1af`. GREEN:15 phone-free tests passed, including fake orchestration,
  pair-reference reset, optional CSVs, label/source/fixture matching, stale and
  unowned-file rejection, saved-write proof and stop-before-cleanup failure.
- Coverage: corrected an initial `coverage --source` file-path invocation that
  collected no data; module-name invocation measured88% of285 statements.
- Verification commands: `python3 -m unittest discover -s tools/performance`,
  `python3 tools/performance/collect_wetland_modes.py --help`,
  `python3 tools/check_docs.py`, and `git diff --check`: PASS. Scoped suite:
  188 tests in1.254s. Help exited successfully. Docs checker:132 Markdown files,
  280 local links,15 ADRs and20 requirements. Diff whitespace check passed.
- No phone/ADB execution, delegates, APK build, measurement or interpretation was
  performed. Physical recovery selection, frame target behavior and visual
  equivalence remain lead execution/review gates. App-field integration is a
  separate worker's responsibility; no claim it is present in an existing APK.
