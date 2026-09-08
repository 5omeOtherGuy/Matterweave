## P00 read-only review — `check_handoff.py` / `test_check_handoff.py`

**Scope:** frozen P00 candidate at `/mnt/bench/matterweave-dev/performance/run-01/p00-candidate-01/tools/performance/check_handoff.py` + `test_check_handoff.py`. Perspective: behavior, stale attempts, CLI input validation. Contract: stdlib read-only validator, `validate_handoff(board, submission, artifact Path)` raises `ValueError` on invalid.

**Checks NOT RUN (read-only):** no execution, no `subprocess`/`unittest` run, no hash verification of candidate SHA256 `699e43ea…` / tests `2d26e3a1…`, no shell. Worker-reported “28 pass” not independently accepted. Findings from static inspection only.

**Files inspected:** both files read in full.

### Findings: none

No concrete candidate meeting the bar for behavior / stale-attempt / CLI-validation defects. Implementation matches contract on all checked paths:

- Exact `schema_version == 1` with `type(x) is int` rejects `True`/`1.0`/`"1"` — correct bool exclusion.
- `run`/`task`/`owner` nonempty-str, `attempt` positive-int with `type is int` rejects `bool`/`0`/`-1` — correct.
- `verified_inactive is not True` rejects `1`/`False`/missing — literal-true attestation correct, no OS-proof claim.
- Digest `fullmatch [0-9a-f]{64}` rejects uppercase/short/long/non-str — correct.
- Stale logic: `run` mismatch, unknown `task`, board `attempt != submission attempt`, `owner` mismatch, `state not in {running,submitted,review-needed}` all raise — replaced/closed correctly rejected.
- Artifact: streaming `hashlib.file_digest` compare, `OSError → ValueError`, mismatch/missing/directory rejected.
- CLI: `_load_json` `read(limit+1)` enforces board ≤8KiB / submission ≤2KiB, non-dict top-level / malformed JSON → `ValueError` → `exit 1` with `error: …`, no traceback path for covered cases.
- Read-only: only `open(..., "rb")` + `read` + hash; tests assert bytes unchanged. Extra metadata ignored as specified.

### Considered but not raised (below bar)

- `validate_handoff(..., artifact=None/123)` would leak `TypeError` from `open()` (`check_handoff.py:99-103`, `except OSError` only) instead of `ValueError`. Not raised: CLI always constructs `Path`, so not a CLI-validation bypass; whether wrong-Python-type counts as “invalid” vs programmer `TypeError` is ambiguous, severity low.
- Deeply-nested JSON within size limits could raise `RecursionError` (not caught by `except (ValueError, UnicodeDecodeError)` in `_load_json`), giving traceback instead of clean error. Not raised: speculative without execution, still nonzero-exit rejection, size-bounded.

### Engineering log

- **Actions Taken:** Read `check_handoff.py` and `test_check_handoff.py` in full; traced `validate_handoff`, `_load_json`, `main` against contract clauses (schema, run/task/attempt/owner, open states, attestation, evidence, digest + streaming hash, stale/closed, size bounds, clean CLI errors, read-only/extra-ignored).
- **Issues & Friction:** No line numbers from read tool — line refs approximate; no shell so SHAs and 28-pass claim unverifiable; `hashlib.file_digest` portability (3.11+) not confirmable read-only.
- **Decisions & Rationale:** Returned no findings rather than filing low-severity/uncertain nits; did not demand process-stop, sandbox, or second-board behavior per contract (“lead separately owns…”).
- **Solutions Applied:** None (read-only leaf, no edits).
- **Insights:** Test suite covers contract well: valid/open-states/extra-ignored/unmutated, stale-replacement fixture explicitly disclaims process proof, closed-states, tampered/missing/bad-digest/directory artifact, bool-attempt, malformed/CLI-clean, oversize bounds. Discriminating gaps if lead wants them: API `artifact=None → ValueError` test; CLI deeply-nested-JSON no-traceback test.

