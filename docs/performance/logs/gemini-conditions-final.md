### Independent Correction Review: `conditions-candidate-02`

#### Review Scope & Verification Baseline
- **Files Verified:** `tools/performance/validate_conditions.py`, `tools/performance/test_validate_conditions.py`
- **Patch Reviewed:** `/mnt/bench/matterweave-dev/performance/run-01/conditions-candidate-02/correction.patch`
- **Frozen SHA-256 Hashes:**
  - `tools/performance/validate_conditions.py`: `113d421b68a05f1050b3a5917f7bffece5585801ac1fb2001519e6543629a8e4`
  - `tools/performance/test_validate_conditions.py`: `594218c50ac5d36e5d645354a9441782c481f9e2b510222579aab6d5c51b16f4`
- **Lead Baseline:** 3 reproduced candidate-01 defects resolved in candidate-02; 96 total tests PASS (`conditions65` + `handoff31`), zero ResourceWarnings, real timestamped device cooling records (`device/cooling-01/health.jsonl`) validate.

---

### Concrete Findings

#### Finding 1: Potential unhandled `OverflowError` in HAL temperature line conversion
- **file:line:** `tools/performance/validate_conditions.py:204`
- **trigger:** A dumpsys `thermalservice` section containing a `Temperature{mValue=...}` entry where `mValue` exceeds IEEE 754 double precision range (e.g. >= 309 digits, like `"9" * 400`) within the 128 KiB line limit.
- **consequence:** `float(match.group(1))` raises `OverflowError: float overflow`. Because neither `parse_thermal_state`, `validate_readiness`, nor `main()` (which catches only `(ValueError, OSError)`) handles `OverflowError`, the CLI terminates with an uncaught traceback instead of a clean `invalid: ...` exit (code 1), violating the fail-closed error contract.
- **evidence:** In candidate-02, overflow guards were added to `_finite_number` (`try: float(value) except OverflowError: raise ValueError`) and `parse_battery_state` (`BATTERY_TEMP_MIN_C * 10 <= tenths <= BATTERY_TEMP_MAX_C * 10` before `/ 10.0`), but `parse_thermal_state` directly passes regex-matched digits (`-?\d+(?:\.\d+)?`) from `mValue` into `float()` without an `OverflowError` handler or integer range check.
- **uncertainty:** Real Android HAL thermal sensors report small decimal floats (e.g. `30.464`), so triggering this requires synthetic, corrupted, or adversarial dumpsys records.
- **check (NOT RUN):** Run CLI against a JSONL row containing `thermalservice` with `Temperature{mValue=999... (400 digits), mType=3, mName=skin, mStatus=0}` to confirm whether an unhandled `OverflowError` traceback is emitted.

*(No further findings identified in correction or affected paths).*

---

### Candidate Assessment on Lead Reproductions

1. **Unbounded `read()` after `stat`:** Fixed. `load_jsonl()` now reads strictly `fh.read(MAX_TOTAL_BYTES + 1)` within a context manager. Files exceeding 1 MiB are rejected without reading unbounded data into memory, and `os.stat` / `import os` are cleanly excised.
2. **Huge `elapsed_s` float overflow CLI traceback:** Fixed. `_finite_number()` wraps `float(value)` with `except OverflowError: raise ValueError(...) from None`, preventing unhandled exceptions in the CLI.
3. **Huge battery integer overflow:** Fixed. `parse_battery_state()` performs integer comparisons (`-200.0 <= tenths <= 800.0`) prior to float division (`tenths / 10.0`).
4. **Test hygiene and coverage additions:** Fixed. Unclosed test file descriptors in `test_valid_file_exit_zero_summary_json` were resolved via `with open(...)`, duplicate key test isolation was verified with specific stderr assertions, and missing coverage for `Dock powered: true` and duplicate/inverted HAL delimiters was established.

---

### Engineering Log

- **Actions Taken:**
  - Inspected `correction.patch` and verified source tree matching frozen candidate-02 SHA-256 hashes.
  - Inspected `conditions-lead-red.log`, `conditions-lead-green.log`, and `conditions-lead-tests.log` to audit lead reproduction and resolution of prior test failures and warnings.
  - Audited all numeric parsing, error translation, boundary conditions, and file I/O lifecycles across `validate_conditions.py` and `test_validate_conditions.py`.
  - Audited device records in `device/cooling-01/health.jsonl` to ensure compliance with parser invariants.
- **Issues & Friction:**
  - Evaluated string-to-float vs int-to-float overflow semantics in Python 3; Python's `float(str)` raises `OverflowError` on numbers exceeding `DBL_MAX`, which is unhandled in `parse_thermal_state:204`.
- **Decisions & Rationale:**
  - Kept review bounded strictly to candidate-02 changes and immediately affected validation paths without revisiting previously rejected candidate designs.
  - Recorded Finding 1 with complete evidence and reproduction profile while noting its synthetic/adversarial trigger profile.
- **Solutions Applied:**
  - None (strictly read-only review per leaf constraints).
- **Insights:**
  - Enforcing input limits at the file boundary (`read(MAX + 1)`) and checking integer bounds before numeric coercion consistently prevents language-runtime arithmetic exceptions from bypassing application error handlers.

Lead owns final acceptance. Stopping after bounded review.
