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
