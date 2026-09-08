# glm-p01-a1 — Device-condition validator slice (P01)

Scope: fail-closed read-only validator of a TWO-MINUTE IDLE READINESS record.
This is a data-condition check, not an app performance benchmark; no energy,
FPS or device-speed claim is made or supported.

Workspace: `/mnt/bench/matterweave-dev/worktrees/performance-p00`
Base: `755620472e8457d8e55760165e759b6d8013c6db` (unrelated ongoing
instrumentation/docs work by other owners left untouched).

## Actions Taken

1. Verified board ownership in `docs/performance/board.json`: `P01-conditions`
   → owner `glm-p01-a1`, paths exactly `tools/performance/validate_conditions.py`,
   `tools/performance/test_validate_conditions.py`,
   `docs/performance/logs/glm-p01-a1.md`. No other worker files touched.
2. Verified dumpsys assumptions against source (read-only):
   `performance/run-01/device/cooling-01/{0..4}-battery.txt` and
   `{0..4}-thermalservice.txt`. Confirmed per sample: all four
   AC/USB/Wireless/Dock powered `false`; `status: 3`; `level: 60`;
   `temperature` 299–300 (tenths C → 29.9–30.0 °C); `IsStatusOverride: false`;
   `Thermal Status: 0`; `Current temperatures from HAL:` section terminated by
   `Current cooling devices from HAL:` with exactly one skin entry
   (30.169–30.464 °C, `mType=3`); a stale `Cached temperatures:` skin entry of
   45.979 °C exists and must never be selected.
3. TDD RED: wrote `test_validate_conditions.py` (60 focused tests, all
   explicitly synthetic fixtures, independently hand-computed expectations)
   first. RED evidence: `ModuleNotFoundError: No module named
   'validate_conditions'` — suite fails before implementation exists.
4. Implemented `validate_conditions.py` (Python stdlib only: argparse, json,
   math, os, re, sys). API `validate_readiness(rows) -> dict` raises
   `ValueError` on any violation; CLI takes one positional `health_jsonl`,
   prints summary JSON + exit 0 when valid, concise `invalid: …` stderr +
   exit 1 otherwise (no traceback). Read-only; no ADB/shell/processes.
5. TDD GREEN: `python3 tools/performance/test_validate_conditions.py` →
   `Ran 60 tests ... OK`, exit 0. (Two intermediate fixture-plumbing errors
   inside the test helpers were fixed; validator needed no contract changes
   after RED.)
6. Real-data convenience check (read-only, lead owns final device use):
   converted cooling-01 dumps to collector rows in `/tmp` with a labeled
   SYNTHETIC 0/30/60/90/120 s timing schedule (only the dump texts are real);
   API and CLI both produce
   `{"sample_count": 5, "elapsed_span_s": 120.0, "battery_min_c": 29.9,
   "battery_max_c": 30.0, "skin_min_c": 30.169, "skin_max_c": 30.464,
   "evidence": "idle_readiness_only"}`, exit 0; sha256 check confirmed source
   files unchanged.
7. Negative sanity check on real data: HAL section removed from one sample →
   rejected ("expected exactly one 'Current temperatures from HAL:' section,
   found 0"); stale cached 45.979 °C is never substituted.

## Issues & Friction

- Sandbox `grep` is a compacting shim that mangles `grep -h` pipelines; used
  small Python one-liners for real-data inspection instead.
- `json.loads` accepts `NaN`/`Infinity` and silently keeps duplicate JSON keys
  by default; both needed explicit rejection hooks (`parse_constant`,
  `object_pairs_hook`) to stay fail-closed.

## Decisions & Rationale

- Skin selection strictly between the exact required boundaries
  `Current temperatures from HAL:` (exactly one) and
  `Current cooling devices from HAL:` (exactly one, after start); any
  `Temperature{...}` line inside that section must fully parse or the row is
  rejected (fail-closed). Cached values are structurally unreachable.
- Battery markers rejected case-insensitively: `updates stopped`,
  `test mode`, `simulat*` — reported charging state is then not trustworthy.
- Required-field duplicates (e.g. two `status:` lines) rejected; missing or
  malformed required data is rejected as unavailable, never defaulted to 0 °C.
- `elapsed_s` must be int/float (bool explicitly invalid), finite, ≥ 0,
  strictly increasing; ≥ 5 samples, span ≥ 120 s, per-gap ≤ 35 s.
- Size bounds: CLI enforces file ≤ 1 MiB and each line ≤ 128 KiB before
  parsing; API enforces the same per-row (serialized) and total budget.
- Extra `data` keys ignored; row-level summary keys: `sample_count`,
  `elapsed_span_s`, `battery_min_c`/`max_c`, `skin_min_c`/`max_c`,
  `evidence: "idle_readiness_only"`.
- No JSON-schema dependency; stdlib regex/int parsing is sufficient for
  thin strict parsing of Android's text dumps (no typed dumpsys parser exists
  in the project; the old collector keeps raw text).

## Solutions Applied

- Duplicate-key and NaN/Infinity JSON rejection via `object_pairs_hook` /
  `parse_constant` in `load_jsonl`.
- CLI wraps load+validate in try/except `(ValueError, OSError)` → single
  concise stderr line, exit 1; argparse handles missing argument (exit 2).
- Unicode/empty-line/oversize line failures all reported as concise
  `line N: …` errors.

## Test & Verification Summary

- RED: `ModuleNotFoundError: No module named 'validate_conditions'` (suite
  fails pre-implementation). PASS/FAIL: 0 pass / 60 fail (import error).
- GREEN: `python3 tools/performance/test_validate_conditions.py` →
  `Ran 60 tests in 0.580s` / `OK`. PASS 60 / FAIL 0.
- `py_compile` on both files: OK.
- Real cooling-01 conversion check (synthetic timing, real dumps): API+CLI
  exit 0 with battery 29.9–30.0 °C, skin 30.169–30.464 °C; source sha256
  unchanged. Cached-only negative check: REJECTED as designed.
- Not run by me: independent reviews, lead device/integration checks, any
  real-device timing (elapsed schedule above was synthetic plumbing).

## Insights

- Android `dumpsys thermalservice` genuinely carries both a stale
  `Cached temperatures:` block and a live `Current temperatures from HAL:`
  block (observed cached skin 45.979 °C vs live 30.169–30.464 °C), so
  boundary-anchored section parsing — not name matching — is what prevents
  cached-value contamination.
- Python's permissive `json` defaults (NaN literals, last-duplicate-key-wins)
  make explicit parse hooks mandatory for fail-closed ingestion.
