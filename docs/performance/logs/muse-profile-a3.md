# Profile validator corrections (P01 attempt 2, review delta) — implementation log

Bounded correction of review findings. Owned files only:
`tools/performance/validate_frame_profile.py`,
`tools/performance/test_validate_frame_profile.py`, plus this log.
Prior log `muse-profile-a2.md` and all other files untouched. No agents, Git
writes, jobs, network, or capture copies. Real captures and the lead repro
were read inputs only.

## Fixes (tests first: RED then GREEN)

1. **unittest, not pytest.** Suite rewritten as `unittest.TestCase` with the
   sibling-file `sys.path` shim (`import validate_frame_profile`), matching
   repo CI (`python3 -m unittest discover -s tools/performance -p
   'test_*.py'`). All fixtures synthetic and explicit. Real private captures
   removed from the suite; checked manually via CLI below.
2. **Late-completion reason.** Second row now uses submitted 2/completed 2 after
   prior submission 1 and asserts the exact `'not before submission'` message.
   The check was also reordered ahead of membership: a same-epoch completion
   at/after this row's own submission is causally impossible even across
   gaps, so it reports the causal reason first; future-epoch, duplicate and
   time-reversed-submission guards are maintained.
3. **Strict CSV.** All parsing goes through `csv.reader(..., strict=True)`
   with `csv.Error` translated to `ValueError` (API and CLI, no traceback
   leak). Verified findings: the unclosed final quoted empty cell the old
   lenient parser ACCEPTED is now rejected (`unexpected end of data`); the
   raw embedded CR the old code leaked as `csv.Error` is now a concise
   `ValueError`. Gemini's NUL/unclosed-quote trigger claim stays rejected —
   under the default dialect the quote was accepted and only CR raised, as
   the lead proved; exact byte-built repro tests for both now assert
   `ValueError`.
4. **Physical line bound.** Raw content length is checked before stripping
   endings (reader now reads +3 to admit content+CRLF). At-bound (16384 B)
   content ACCEPTED with LF and CRLF; over-bound (16385 B) REJECTED with LF
   and CRLF; `raw_sha256` still covers every raw byte (asserted against an
   independent hash of the file bytes).
5. **Summary semantics, fixed deliberately.** `unmatched_completions` now
   means an OBSERVED completion whose earlier submission row is missing (only
   allowed when earlier draw attempts are genuinely missing; otherwise still
   an invalid orphan). New `uncompleted_submissions` counts
   submitted-minus-completed. Verified by hand: draw 1/submit 1 +
   draw 3/submit 3/complete 2 → accepted with missing_attempts 1,
   unmatched_completions 1, uncompleted_submissions 2; complete single
   presented row → 0/1. No join is fabricated: gap completions join nothing
   and only feed duplicate detection.
6. **Missingness per column.** `missing_cells` is now a dict over the 33
   optional columns (unavailable clocks distinguishable from absent GPU
   data); `missing_cells_total` carries the sum. Tests assert independent
   hand-computed field values and the total.
7. **Descriptor rejection.** `validate_profile` rejects int/bool (and other
   non-path types via `os.fspath`) with `ValueError` before `open` is
   touched; a mocked-`open` test asserts not-called for 0/1/False/True/None.
   Read `OSError` stays translated to API `ValueError` as documented.
8. Removed the now-unused parsed duration/shadow locals (validation calls
   kept); reused only csv/pathlib-equivalent (`os.fspath`)/unittest.

## Verification

- RED: new unittest suite against the old validator → 7 failures + 10
  errors, every one mapping to an old behavior listed above (the exact
  not-before-submission reason test passed under both orders, pinning the
  message either way).
- GREEN: `TMPDIR=…/run-01/scratch python3 -m unittest discover -s
  tools/performance -p 'test_validate_frame_profile.py'` → **33 tests, OK**.
  Full directory discovery → **129 tests, OK**. `python3 tools/check_docs.py`
  → PASS (72 files).
- Manual CLI, inputs unchanged (mtime/size asserted before/after):
  device-868 → VALID 1000 rows epochs [1] missing 0 unmatched **0**
  uncompleted **1**; device-134 → VALID 581 rows epochs [1, 2, 3] missing 0
  unmatched **0** uncompleted **3**; host-smoke-02 → VALID 90 rows epochs
  [1, 2] missing 0 unmatched **0** uncompleted **2**.
- Lead repro (read-only): complete-single-row VALID (0/1); line-cap-plus-one
  VALID; missing-submission-row VALID (2 rows, 1/1/2 as specified);
  embedded-cr and unterminated-quote REJECTED as concise `ValueError`.

## Remaining gaps

- Acceptance and serial re-review are lead-owned. Judgment calls to confirm:
  gap-allowed completions use cumulative prior missing attempts (not
  per-epoch gap tracing); a same-epoch completion on a no-submission row
  with gaps is accepted without an upper-bound check.
- Original a2 evidence (`muse-profile-a2.md`, scalar `missing_cells`,
  submitted-minus-completed meaning) is preserved in that log for history;
  this log supersedes its semantics, not its raw results.
