# Engineering log w_b8d9841b

Worker: `opus-medium` · outcome: Category: implement · run: `/mnt/bench/matterweave-dev/completion-20260912/e2-collector-opus`

Outcome: make the E2 same-build capture ON/OFF qualification actually
collectible. Before this change `collect_wetland_pair.py` always wrote a
profile request and always required exactly one capture per trial, so the
collection mode that
[docs/performance/measurement-qualification.md](../measurement-qualification.md)
assumes did not exist.

- **Actions Taken:**
  - Added `--mode {paired,capture-on-off}` to
    `tools/performance/collect_wetland_pair.py`. `paired` is the default and
    keeps the candidate/reference contract; `capture-on-off` takes a single
    `--apk` and plans alternating OFF/ON, ON/OFF, OFF/ON trials
    (`capture_state_plan`).
  - Added `check_identical_build`, applied after every ON/OFF trial, so the
    batch aborts unless the *installed* APK hash is byte-identical across
    trials and equals the frozen manifest hash.
  - Made the profile request creation conditional on `capture_state == "on"`
    and parameterised `finish_and_pull` on the capture state: OFF creates no
    request, reports `profile_request_consumed: null` instead of a fake
    "consumed" flag, expects zero new `frame-profile*` files, records any that
    appear in `unexpected-captures.json` and rejects the trial while leaving
    them on the device, and rejects (never consumes or deletes) a foreign
    `profile-frames.txt` that appears mid-trial.
  - Recorded `capture_state` explicitly per run in `manifest.json`,
    `trial.json`, `collection-complete.json` and `run-complete.json`; paired
    trials are explicitly `"on"`.
  - `analyze_wetland_pairs.py`: reads the recorded `capture_state` instead of
    inferring one, accepts a trial with no capture CSV only when the state is
    `off`, pairs by `capture_state` when a pair holds one build in two states
    (`pair_members`, delta key `capture_on_minus_off`), skips CSV-only match
    checks when one side has no CSV, requires an ON/OFF pair to share one APK
    hash, and emits a per-trial `sampling` block.
  - Documented the ON/OFF collection mode, an exact runnable command, and a
    "What has no automatic source" subsection in
    `docs/performance/measurement-qualification.md`.
  - Tests: CLI-contract tests now parse real argv through
    `build_parser()`/`check_args()` (replacing a hand-built stand-in object),
    plus new tests for the ON/OFF plan, build-identity check, OFF finish
    behaviour, unexpected-capture rejection, foreign-request preservation, and
    the analyzer's capture-state pairing.

- **Issues & Friction:**
  - `check_args` previously ran against a fake args object in the tests, so
    adding real CLI arguments broke it. Rewriting those tests to go through the
    parser is closer to the actual contract and removes the divergence.
  - `finish_and_pull`'s `consumed = PROFILE_REQUEST_NAME not in after` is
    `True` for an OFF trial for the wrong reason (nothing was ever written).
    Reporting it as `null` in OFF avoids a field that reads like evidence of a
    consumed request.
  - The qualification doc's `missing_observation_count` had no producing tool.
    See the decision below.

- **Decisions & Rationale:**
  - One collector with a bounded mode flag rather than a second wrapper script:
    every ownership, readiness, thermal-matching, fixture, foreground/unplugged
    and cleanup rule is shared verbatim, and a wrapper would have had to
    re-enter `run_trial` anyway.
  - ON/OFF pairs alternate within a pair exactly like the build pairs, so a
    monotonic device drift cannot align with one capture state.
  - The analyzer keys pairs off recorded data (`variant`, then `capture_state`)
    and never guesses the compared factor; an old batch without `capture_state`
    keeps the previous one-capture requirement.
  - Missing-sample accounting: the collector polls compositor dumps best effort
    at ~0.5 s and schedules no fixed observation count, so there is no honest
    "scheduled minus obtained" number. The analyzer already refuses any dump
    with a nonzero exit or non-monotonic host clock, so a reduced trial has
    zero discarded dumps and a rejected trial produces no row at all. The
    `sampling` block states exactly that and the doc tells the lead to
    transcribe `compositor_dumps_discarded` rather than count by hand. No count
    was invented.

- **Solutions Applied:** see Actions Taken; all changes are confined to the
  owned paths.

- **Insights:**
  - "Absence of a capture file" is only meaningful next to a recorded intent.
    Without an explicit per-run `capture_state`, an OFF run and a failed ON run
    are indistinguishable downstream, which is precisely the ambiguity the
    qualification contract forbids.
  - Ownership discipline made the OFF state cheap to implement safely: because
    only created files are ever owned, an unexpected profile or foreign request
    is preserved by the existing cleanup path with no extra code.

## Verification

| Criterion | Verification method | Owner | Result |
| --- | --- | --- | --- |
| Explicit same-build ON/OFF collection using independent common measurements | `collect_wetland_pair.py --help`; `CaptureModeArgumentTest`, `CaptureStatePlanTest`, `IdenticalBuildTest`; SF/`/proc/<pid>/stat` collection untouched and shared by both states | worker | PASS |
| OFF omits request and rejects unexpected profiles; foreign requests and save restore preserved | `CaptureOffFinishTest` (5 tests) plus the pre-existing `OwnershipCleanupTest` | worker | PASS |
| Default paired collector/analyzer remains compatible | `python3 -m unittest discover -s tools/performance -p 'test_*.py'` → **237 tests, OK**; paired dry run writes `mode: paired`, `compared_factor: variant`, all trials `capture_state: on` | worker | PASS |
| Qualification doc gives exact runnable command and truthful missing metadata handling | `python3 tools/check_docs.py` → PASS (166 files, 563 links); doc command matches the `--help` contract | worker | PASS |
| Actual matched Android qualification and review | lead | lead | NOT RUN |

Host-only. No phone was used, no APK was built and no measurement was taken:
E2 is **not** complete. The ON/OFF mode has never been executed against a
device, so its device-side behaviour (profile-request handling by the app in
the OFF state in particular) is unverified.
