### Independent Review: ON/OFF Collector & Analyzer Extension (Diff `22070ca..40a0844`)

**Scope & Verification Context:**
- Worktree: `/mnt/bench/matterweave-dev/worktrees/collector-review-20260912` (frozen commit `40a0844`)
- Diff: `/mnt/bench/matterweave-dev/completion-20260912/collector-review.diff`
- Focus: user-save/request ownership/cleanup, OFF semantics, same-APK verification, common independent timings, missing data & sample accounting, run chronology vs paired signs.
- Authority: Read-only leaf reviewer. No edits/builds/device invocation performed.

---

### Key Mechanism Assessment

1. **User-Save / Request Ownership & Cleanup:**
   - Worker writes only to slot 127 (`files/wetland-session.json.recovery-127.json`). Base save (`wetland-session.json`) is strictly preflighted to ensure it is invalid for generator 2, forcing `load_recovering` to pick slot 127 without touching the base save or user `world.json`.
   - `Ownership` tracking strictly records `FIXTURE_REMOTE_NAME` and `PROFILE_REQUEST_NAME` before creation. `final_cleanup` removes only files in `ownership.owned`.
   - In OFF mode, `PROFILE_REQUEST_NAME` is never claimed or written. If a foreign request exists or appears, it is neither owned nor deleted, and the run fails closed (`finish_and_pull`).

2. **OFF Semantics:**
   - In `--mode capture-on-off`, OFF trials write no profile request, expect zero capture CSVs, and treat any newly appeared `frame-profile*` CSV as a disqualifying unexpected capture recorded in `unexpected-captures.json` without deletion.
   - Analyzer enforces `captures == []` for OFF trials (`analyze_wetland_pairs.py:235-237`), failing closed if a CSV is present.

3. **Same-APK Verification:**
   - Collector re-verifies `*-build.json` against the local binary, reinstalls before each trial via `pm install -r -d`, computes `sha256sum` of the remote installed APK via `pm path`, and calls `check_identical_build(installs)` after every trial to enforce exact byte equality across the batch.
   - Analyzer checks `pair_mismatches` (`apk_sha256` matching when `capture_state` differs).
   - Qualifier requires top-level `build` digest and metadata to match across all runs.

4. **Common Independent Timings:**
   - Timings in both ON and OFF trials are collected via identical `dumpsys SurfaceFlinger --latency` polling and `/proc/<pid>/stat` CPU snapshots across identical warmup and measurement windows, remaining fully decoupled from app capture CSV generation.

5. **Missing Data & Sample Accounting:**
   - Analyzer explicitly emits `sampling.compositor_dumps_discarded: 0` because any dump with a nonzero exit code or nonmonotonic timestamp causes `presentation_intervals` to raise `ValueError`, failing the trial completely rather than discarding intermediate frames. Hand-transcription into `missing_observation_count: 0` is accurate.

6. **Actual Run Chronology vs. Paired Signs:**
   - Qualifier `_sequence` (`qualify_measurements.py:437-448`) pairs consecutive chronological trials (`zip(runs[0::2], runs[1::2])`) without reordering.
   - Counterbalanced ordering across pairs (e.g. OFF/ON, ON/OFF, OFF/ON) is permitted. `capture_overhead` correctly computes `on - off` for every pair regardless of within-pair order (`first` vs `second`), ensuring consistent overhead signs while allowing thermal drift to cancel out in the average.
   - Qualifier `resolution` handles zero-noise and zero-overhead without division by zero, yielding `"no observed difference"`.

---

### Findings (3 Proven Low/Diagnostic Observations, 0 Fatal Defects)

#### Finding 1: `pair_members` lacks pair-cardinality validation (`len(members) == 2`)
- **File & Line:** `tools/performance/analyze_wetland_pairs.py:277-285`
- **Trigger:** An input batch directory containing more than two trial directories for the same pair index (e.g., from aborted and resumed trial runs where older directories remained present).
- **Consequence:** `by_state = {t['capture_state']: t for t in members}` and `by_variant = {t['variant']: t for t in members}` dict comprehensions silently overwrite earlier trials with later ones, resolving a pair from an arbitrary subset of trials rather than rejecting the ambiguous pair.
- **Uncertainty:** Low operational impact during clean runs because `main()` iterates over `manifest['plan']`, which contains exactly two trials per pair.

#### Finding 2: Intra-trial stability checks skipped for ON trials in ON/OFF comparisons
- **File & Line:** `tools/performance/analyze_wetland_pairs.py:303-306`
- **Trigger:** Execution of `analyze_wetland_pairs.py` on a same-build capture ON/OFF pair.
- **Consequence:** In `pair_mismatches`, the whole-capture block is guarded by `if a['app_whole_capture'] and b['app_whole_capture']:`. Because the OFF trial `a` has `app_whole_capture is None`, the loop checking `len(sets[row]) == 1` (shadow map size, voxel body totals changing mid-run) is skipped for the ON trial `b`. Mid-run changes during the ON trial's capture are not caught by `pair_mismatches`.
- **Uncertainty:** Low risk of silent corruption because `collect_wetland_pair.py` already passes the ON CSV through `validate_profile` schema validation and verifies end-of-trial body counts in `session-after.json`.

#### Finding 3: Diagnostic mismatch label if candidate/reference pair has mixed capture states
- **File & Line:** `tools/performance/analyze_wetland_pairs.py:278-280`, `294-295`
- **Trigger:** A candidate vs reference paired run where one trial had `capture_state: "off"` and the other `"on"`.
- **Consequence:** `pair_members` resolves this as `candidate_minus_reference` (since `by_variant` has both variants), but `pair_mismatches` records `'capture on/off pair does not share one build'` because `a['apk']['apk_sha256'] != b['apk']['apk_sha256']`. The error correctly blocks delta computation, but the diagnostic reason misattributes the failure to a same-build comparison failure instead of a capture-state mismatch between build variants.
- **Uncertainty:** Collector `trial_plan` hardcodes `"capture_state": "on"` for all paired-mode trials, so this can only occur with manually crafted or corrupted metadata.

---

### Checks NOT RUN
- Physical ADB phone execution against OnePlus 13 (Lead-owned).
- End-to-end device execution of `collect_wetland_pair.py --mode capture-on-off`.
- On-device measurement qualification generation.

### Short Engineering Log
- Inspected full diff `/mnt/bench/matterweave-dev/completion-20260912/collector-review.diff` across documentation, collector, analyzer, qualifier, and test suites.
- Verified ownership lifecycle (`preflight_files`, `run_trial`, `finish_and_pull`, `final_cleanup`) and confirmed base save isolation.
- Verified OFF trial error paths for unexpected CSVs and foreign profile requests.
- Traced APK hashing and multi-trial identity verification logic.
- Analyzed sample accounting in `presentation_intervals` and qualifier sample validation.
- Checked pairing arithmetic, counterbalancing sign cancellations, and zero-difference verdict branches.
