# Wave 1 review triage

Frozen source `22070ca`, base `704bb4a`. Independent [Muse review](reviews/completion-wave1-muse-20260912.md) and [Gemini review](reviews/completion-wave1-gemini-20260912.md) were obtained without sharing findings. The lead read source and performed/assigned reproductions; agreement alone is not acceptance. Hy4's narrower cadence review timed out after 240 s and its final-only 120 s continuation also timed out; no review result is claimed.

| Candidate | Lead disposition | Evidence / next action |
| --- | --- | --- |
| Muse F1: wrong unload generation | Confirmed from producer and consumer source; pre-existing, delivery-blocking | Producer sends next generation, consumer compares existing generation; corrective worker owns real PCM/unload/reuse regression and repair. |
| Muse F2 / Gemini 1: failed pause with one queue slot | Confirmed pre-existing in `704bb4a`; delivery-blocking | Suspend consumes last slot; ignored compensation fails; private backend failure regression/repair assigned. |
| Muse F3 / Gemini 4: zero difference exceeds zero noise | Reproduced and fixed at `959faab` | New identical-repeat test failed before fix; 220 performance tests pass after fix. Constant nonzero difference at zero noise tested separately. |
| Gemini 2: stale disconnect across failed start | Source-supported; corrective worker verifying | Clear old error state only after joining old callbacks and before opening replacement; retain new-stream errors. |
| Gemini 3: disconnected stream properties | Source-supported mock/native divergence; corrective worker verifying | AAudio properties must reflect shared disconnect before poll; regression/repair assigned. |
| Gemini 5: scheduler-state OR assertion | Confirmed redundant; removed at `d91058b` | Actual blocking, tracked-job, explicitly buffered-result and post-publication movement checks remain; no requirement that worker run slowly. |
| Gemini 6: teleported gate uses low wall | Rejected as a defect in this test | Test checks publication before the next physics step, using current occupied AABB and unchanged collider count. It does not claim traversal prevention; autostep cannot invalidate its pre-step overlap assertion. Separate tall-wall movement regression covers traversal. |
| Lead: callback epoch is not unload acknowledgment | Source-supported race; deterministic reproduction assigned | A callback can drain queue before enqueue, then publish an epoch without applying the unload. PCM reuse and pending play liveness must follow actual command completion, not guessed callback progress. |

Initial integrated checks: 477 Rust tests passed (3 existing ignored gates); strict workspace Clippy and fmt pass. Physical OnePlus 13: 11 ARM64 cadence tests passed in each of 3 runs; AAudio diagnostic passed 4 runs × 20 paused-recreation/resume/shutdown cycles at `22070ca`. Those results do not accept subsequent audio corrections; affected checks and independent review must be repeated at the corrected frozen revision.

Supervisor log-check friction: relative `--log` paths for the first two workers resolved in the lead checkout rather than worker worktrees; source-controlled worker logs were read directly and the unused scaffolds removed. The absolute-path audio log also received a heading-parser warning despite containing all five populated headings. These warnings are recorded, not treated as missing engineering evidence or silently relabeled as passed supervisor checks.

Corrective checkpoint `da2e4fc`: worker RED commit `540b4d4` and repair
`c277f15` (integrated as `0064843` / `da2e4fc`) reproduce and repair
unload-generation, failed-pause compensation, command/voice acknowledgment,
and stale/new backend-error issues. Worker package suite: 43 tests PASS,
strict scoped Clippy/fmt PASS, 20 host diagnostic cycles PASS. These remain
candidates pending corrective independent review and Android reruns.

The [independent ownership consultation](reviews/audio-alias-oracle-20260912.md)
confirmed the separate live-core mutable-reference defect. Disjoint sample
indices alone do not justify registration through a mixer simultaneously
borrowed mutably by AAudio. A separately shared fixed PCM allocation with
narrow cell access is under implementation; no overall safety acceptance yet.

Collector frozen `40a0844`: [Muse review](reviews/collector-review-muse-20260912.md)
found no source-proven defects, Gemini review pending. Lead reproduced a
real cross-tool mismatch: counterbalanced OFF/ON, ON/OFF pairs were rejected
by the qualifier's global-alternation rule. `40a0844` fixes chronological
pair validation with a red/green regression; full Python suite 238 PASS.
Phone preparation also found the historical generator-2-only collector
incompatible with current generator 3. A bounded compatibility repair is
running; no current-build overhead or repeatability result exists yet.

[Gemini collector review](reviews/collector-review-gemini-20260912.md)
identified missing pair cardinality validation and ON-only capture stability
checks. Lead reproduced both: four failing assertions across two tests, then
240 full Python tests PASS after fixing. The review's mixed-build/state label
observation already rejects the malformed comparison; its reported diagnostic
is accurate (different APKs cannot establish capture overhead), so no behavioral
change is needed. Generator compatibility and final corrective reviews remain open.

Final collector source `e436634`: independent [Muse](reviews/collector-final-muse-20260912.md)
and [Gemini](reviews/collector-final-gemini-20260912.md) corrective reviews report
no findings. Lead full suite: 251 Python tests PASS. Source changes accepted;
current-build mobile overhead/noise qualification remains NOT RUN, independently
of that source acceptance. Connected phone keyguard currently blocks graphics
operation; successful installation is not treated as execution evidence.

Final ownership/lifetime source `b58527f`: independent [Muse](reviews/audio-final-muse-20260912.md)
and [Gemini](reviews/audio-final-gemini-20260912.md) reviews found no remaining
PCM/core ownership defect. Gemini identified one additional pause-intent failure:
`suspend()` returned success when recovery had failed without recording suspension.
Lead regression failed with “pause intent lost during recovery”; `a999fd0` fixes
the guard and clears stale Resume bookkeeping on a fresh successful suspension.
A second regression covers idempotent suspension after a failed resume.
49 package tests and 494 workspace tests PASS (3 existing ignored gates).
[Muse narrow corrective review](reviews/audio-pause-muse-20260912.md) has no findings;
[Gemini narrow review](reviews/audio-pause-gemini-20260912.md) also has no findings.
Final source `a999fd0`: 49 ARM64 correctness tests and 4 × 20 real AAudio lifecycle
cycles PASS. Lead accepts E1 audio functional corrections; audibility, actual
headset disconnect, Miri and both-sample integration remain open.
