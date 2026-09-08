# Engineering log — muse-p00-a2 (P00-handoff, attempt 2)

Run `performance-01`, task `P00-handoff`, attempt 2, owner `muse-p00-a2`.
Board: `docs/performance/board.json` (lead-written, attempt 2, state `running`).
Owned paths only: `tools/performance/check_handoff.py`,
`tools/performance/test_check_handoff.py`, this log. No commits, no agents
spawned, no detached jobs, no phone access, no other edits.

## Actions Taken

- Read the updated board: attempt 2, owner `muse-p00-a2`, state `running`,
  same frozen handoff interface v1 and disjoint paths.
- Verified attempt-1 partial work preserved byte-identical: workspace hashes
  match `cancelled-a1-partial` and the lead manifest
  `run-01/cancellation-partial-sha256.json`
  (`check_handoff.py` `699e43ea…88faa`, `test_check_handoff.py` `2d26e3a1…b9`).
- Inspected both preserved files in full; implementation and suite were
  complete, so attempt 2 continued with verification, not a rewrite.
- Reran the focused suite GREEN: 28/28 OK
  (`PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/performance -p "test_check_handoff.py"`).
- Ran CLI end-to-end against the real lead board with a scratch artifact in
  `/tmp` (removed afterwards): valid attempt-2 submission → exit 0
  (`handoff valid`); stale attempt-1 submission → exit 1 with concise
  `stale or unknown attempt` diagnostic; malformed board JSON → exit 1 with
  concise diagnostic; no traceback in any rejection; scratch artifact hash
  unchanged.
- Confirmed stdlib-only imports (`argparse`, `hashlib`, `json`, `re`, `sys`,
  `pathlib` in the validator; plus `subprocess`, `tempfile`, `unittest` in
  tests; the one `check_handoff` import is the module under test, not a
  dependency). Removed `__pycache__` byproducts so only the two owned source
  files remain under `tools/performance/`.

## Issues & Friction

- Attempt 1 was cancelled mid-session (see Cancellation below) after its
  files existed; no work was lost, but RED→GREEN continuity depended on the
  lead's preserved copies and hash manifest, which checked out exactly.
- This machine's `grep` is wrapped (`rtk grep`); used the file-search tool
  for import inspection instead. Minor, no code impact.
- `unittest discover` from the worktree root writes `__pycache__`; reran
  with `PYTHONDONTWRITEBYTECODE=1` and deleted the directory.

## Decisions & Rationale

- Made no code changes in attempt 2: the preserved validator already meets
  the frozen v1 interface and every DoD bullet; edits for churn alone would
  violate smallest-correct-change.
- Kept test-board defaults at generic fixtures (attempt 1 / `muse-p00-a1`)
  because the suite already covers attempt-2 replacement explicitly
  (`TestCancellationFixture`, stale-attempt test); real-board attempt-2
  ownership is proven by the CLI check against the actual board JSON.
- `verified_inactive=true` is treated strictly as `is True` and documented
  as manual lead attestation, never a claim of OS proof. The validator
  performs no Git operations and kills no processes.

## Solutions Applied

- Validator (`check_handoff.py`, ~5.8K): `validate_handoff(board, submission,
  artifact)` raises `ValueError` on schema/run/task/attempt/owner mismatch,
  non-open states (only `running`/`submitted`/`review-needed` pass), bad
  digest format, artifact missing/mismatch (streamed via
  `hashlib.file_digest`, never loaded into memory), and absent/type-invalid
  fields (`type(x) is int` guards so `bool` attempts fail). CLI bounds board
  JSON to 8 KiB and submission JSON to 2 KiB, rejects non-object top levels
  gracefully, prints one `error:` line to stderr, exits 1, never tracebacks.
- Suite (`test_check_handoff.py`, 28 tests): CLI + API coverage for valid
  handoff, stale attempt after replacement, wrong owner/run, inactive
  false/missing/non-bool-truthy, closed states
  (`closed`/`blocked`/`replaced`/`accepted`/`revoked`), changed/missing/
  directory artifact, malformed JSON/types, size bounds, file non-mutation,
  and the cancellation/replacement fixture (partial bytes + hash preserved,
  attempt-1 rejected, attempt-2 validates against the same bytes).

## Insights

- The small-stateless-validator approach composes well with cancellation:
  replacement is just a board row change, and staleness falls out of the
  attempt/owner equality check with zero new machinery.
- Strict `is True` / `type is int` checks are the only non-obvious code;
  both exist because JSON truthiness (`1`, `"true"`) must not pass as
  attestation or attempt numbers.

## Verification results (actual)

- RED (attempt 1, same session lineage, pre-implementation):
  `python3 -m unittest tools.performance.test_check_handoff` →
  `Ran 28 tests … FAILED (failures=1, errors=23)`:
  `ModuleNotFoundError: No module named 'check_handoff'` (23 API tests) and
  CLI `can't open file … check_handoff.py` (1 failure). Three further
  failures after implementation were test-helper bugs (missing `**extra`,
  `None` digest masked by default), fixed in the tests, not the validator.
- GREEN (attempt 2, this turn): same suite via discover → `Ran 28 tests … OK`.
- CLI vs real board: valid attempt-2 → exit 0; stale attempt-1 → exit 1,
  `error: invalid handoff: stale or unknown attempt: board attempt does not
  match`; malformed board → exit 1, `error: invalid handoff: board JSON is
  not valid JSON: …`; no `Traceback` in any stderr; scratch artifact
  untouched (since removed).
- NOT RUN (lead-owned): independent Muse/Gemini review, lead inspection,
  real process-inactivity/device checks, phone readiness.

## Limitations

- This fixture proves the validator rejects stale attempt-1 output and
  preserves partial bytes; it does NOT prove process termination. Real
  cancellation evidence (runner 359107 / session 359108 SIGTERM) is lead's
  record in `docs/performance/p00.md`, not mine.
- Old attempt-1 output cannot be admitted as attempt-2: the suite rejects
  cross-attempt reuse, and no attempt-1 submission was validated here.
- `verified_inactive` is an attestation field, not OS proof; enforcement of
  actual quiescence stays with the lead's manual review step.
