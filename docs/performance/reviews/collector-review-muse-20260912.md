No findings.

## Review scope

Independent read-only leaf at `40a0844` in `/mnt/bench/matterweave-dev/worktrees/collector-review-20260912`, diff `completion-20260912/collector-review.diff` (22070ca → 40a0844). No worker logs or other reviewer output read. No edits, builds, device work, or acceptance.

## What was checked (source-proven, no defect meeting the bar)

- **User-save / request ownership / cleanup** — `collect_wetland_pair.py:489` (`preflight_files` rejects pre-existing request/fixture/outranking slots, claims fixture before write); `run_trial` (`:798`) rejects a pre-existing `profile-request.txt` in both modes and claims it only in ON; `finish_and_pull` (`:686`) never deletes a foreign request; `final_cleanup` (`:866`) removes only `ownership.owned`. OFF-mode `CaptureOffFinishTest` covers leave-in-place behavior.
- **OFF semantics** — OFF creates no request (`:809`), expects zero new `frame-profile*` files, records `unexpected-captures.json` and raises leaving files on device (`:686`); analyzer rejects an OFF trial carrying a capture (`analyze_wetland_pairs.py:230`); `capture_state` is read, never inferred, in collector, analyzer, and qualifier docs.
- **Same-APK verification** — per-trial `install_and_verify` (`:430`) hashes the installed APK against the frozen manifest; cross-trial `check_identical_build` (`:178`, invoked `:1060`) aborts on any installed/expected divergence. Redundant by design, not contradictory.
- **Common independent timings** — `collect_window` (`:608`) is mode-independent (SurfaceFlinger + `/proc/<pid>/stat` + health in every trial); analyzer derives `presentation`/`process_cpu` from those two sources only, `app_whole_capture=None` for OFF, and `pair_mismatches` skips CSV-field comparison unless both sides have a capture.
- **Missing data / sample accounting** — `presentation_intervals` raises on any nonzero-exit or non-monotonic dump, and `collect_window` aborts the trial on invalid presentation history, so a reduced trial entailed zero discards; the hardcoded `compositor_dumps_discarded: 0` with its note plus the doc redefinition ("no scheduled count; do not count by hand") is consistent, not a miscount.
- **Chronology vs paired signs** — collector emits counterbalanced OFF/ON, ON/OFF (`:160`); qualifier `_sequence` (`qualify_measurements.py:426`) pairs chronologically without sorting and `capture_overhead` signs on-minus-off regardless of within-pair order; analyzer `pair_members` (`analyze_wetland_pairs.py:271`) returns `(off, on)` with `on − off` deltas. Signs agree across both consumers. Zero-noise verdict fix (`mean_delta == 0` → "no observed difference") corrects the old behavior where identical repeats fell through to "exceeds".
- Residual uncertainty (not a finding): the qualifier trusts transcribed run order for chronology and cannot cross-check it against per-trial timestamps; transcription correctness remains lead-owned as the contract states.

## Checks NOT RUN

No builds, no unit tests, no device execution (per task authority; collection mode is stated NOT RUN against a phone in the docs).
