## Read-only review — `validate_conditions.py` + `test_validate_conditions.py`

Scope: `performance/run-01/conditions-candidate-01/tools/performance/validate_conditions.py`, `test_validate_conditions.py`, `docs/performance/logs/glm-p01-a1.md`.
Frozen hashes provided, **match NOT independently verified** — no hash/shell tool in this read-only leaf.
**Checks NOT RUN (read-only).** Worker claim `60 tests PASS` **not independently accepted**.
Real-dump check in worker log used explicitly **synthetic 0/30/60/90/120s schedule** — NOT proof of real two-minute readiness; lead rerun with actual timestamped samples required.
No hardware calibration / energy / FPS / auth claim made or supported — module docstring and log correctly scope to `idle_readiness_only`.

Contract checked: ≥5 samples, finite strictly-increasing nonneg `elapsed_s` (bool invalid), span ≥120s, gaps ≤35s; per-row raw `battery` (4× powered false required, status 3, level 0..100, temp −20..80°C, reject stopped/simulated) + `thermalservice` (`IsStatusOverride` false, `Thermal Status` 0, exactly one HAL skin −20..100°C only between the two required HAL boundaries, never cached); cross-sample battery ≤1°C, skin ≤2°C; duplicate/missing/malformed rejected; 1MiB/128KiB bounds; NaN/nonfinite rejected; extra nonconflicting metadata ignored; API `ValueError`, CLI concise nonzero, inputs unchanged.

### Candidate findings — none valid as contract blockers

**C1 — Dead constants, no behavior effect — `validate_conditions.py:66,72`**
Trigger: `BATTERY_REQUIRED_INT_KEYS` and `_DECIMAL_RE` defined, never referenced.
Consequence: none — each battery field individually parsed/required; skin numeric path via `_TEMPERATURE_LINE_RE` + `float()` + `isfinite`.
Evidence: definition present; all call sites use `_parse_scalar_field`/`_parse_int_field`/`_TEMPERATURE_LINE_RE`.
Uncertainty: low.
Discriminating check (NOT RUN): usage search already done via read/grep; removal-only diff + suite rerun would confirm.
Verdict: **not valid** — maintainability nit only.

**C2 — `load_jsonl` total bound pre-checked via `stat`, no post-read `len(raw)` check — `validate_conditions.py:321,327`**
Trigger: `os.stat` size check, then unbounded `fh.read()`; total re-checked only later on serialized rows in `validate_readiness`.
Consequence: concurrent file growth between stat and read could over-read memory; no false-accept — oversize still rejected downstream (serialized total + per-line raw checks).
Evidence: no `len(raw) > MAX_TOTAL_BYTES` after read in `load_jsonl`.
Uncertainty: medium — requires racing writer; local-file validator threat low.
Discriminating check (NOT RUN): concurrent-grow or directly constructed >1MiB file CLI test asserting concise reject without OOM.
Verdict: **not valid** as acceptance blocker — hardening note only.

**C3 — CLI duplicate-keys test weakly isolated — `test_validate_conditions.py:545`**
Trigger: doctored duplicate-`elapsed_s` line appended out of order (rows[1:]+dup); assertion only nonzero + no traceback.
Consequence: if `object_pairs_hook` regressed, test still passes via timing-order failure — wrong-reason pass. Validator itself correctly installs `parse_constant` + `object_pairs_hook` (`validate_conditions.py:354`).
Evidence: test code read; validator hook code read.
Uncertainty: low.
Discriminating check (NOT RUN): in-order same-value duplicate-keys fixture asserting load-stage reject message.
Verdict: **not valid** against validator — test-strength nit only.

**C4 — API size accounting on re-serialized JSON vs CLI raw bytes — `validate_conditions.py:255-265`**
Trigger: API sums `len(json.dumps(row))`, CLI checks `st_size` + raw line length.
Consequence: fail-closed direction only (possible over-reject on exotic whitespace/unicode); no false-accept since CLI path enforces both.
Evidence: code comparison.
Uncertainty: low.
Discriminating check (NOT RUN): compact-vs-padded / non-ASCII same-row CLI-vs-API comparison.
Verdict: **not valid** — no contract violation.

No other concrete contract deviations confirmed on read.

### Engineering log (for `docs/performance/logs/muse-conditions-review.md`)

Actions Taken: read-only read of validator, suite, `sha256.json`, worker log; mapped contract clauses to code paths (timing, battery, thermal HAL boundaries, stability, size/NaN/duplicates, API/CLI read-only behavior); grep for hooks, bounds, duplicate handling.
Issues & Friction: no shell/hash/execution available so hashes and 60-test claim unverified; line numbers from read/grep only; real device dumps not re-inspected in this leaf.
Decisions & Rationale: report only fail-closed contract violations as blockers; downgraded dead code, TOCTOU read sizing, test isolation, serialization accounting to non-blocking notes with explicit uncertainty and un-run discriminating checks.
Solutions Applied: none — review leaf, no edits.
Insights: main correctness hinges are exact HAL boundary anchoring (cached 45.979°C contamination prevention) and strict `json` hooks for NaN/duplicate keys — both present; boundary timing (`<120` reject / `>35` reject) and range (`>` reject) correctly implement inclusive limits.

