## P00 attempt-2 delta review — candidate-02

**Files inspected:** `tools/performance/check_handoff.py`, `tools/performance/test_check_handoff.py` under `/mnt/bench/matterweave-dev/performance/run-01/p00-candidate-02`.

**NOT RUN (read-only):** no execution, shell, hash verification, or test run. Reported hashes `ea537091…` / `4081bf8a…` and “31 pass / RED showed 3 failures” not independently verified. Lead owns final acceptance.

### Correction verified

- `check_handoff.py` adds pre-`open` guard: `if not isinstance(artifact, Path): raise ValueError(...)`. This closes the prior `TypeError` escape (`None`/`[]`/`int`) and specifically prevents `open(0,"rb")` from touching stdin fd 0. Placement after board/submission checks, before hash — preserves existing ordering.
- New test `test_invalid_artifact_type_rejected_without_opening_descriptors` patches `builtins.open` with `AssertionError` for `(None, [], 0)` — directly discriminates the fix.

### Changed-invariant assessment

- **Strictening, contract-aligned:** `str`/`bytes` artifacts previously tolerated by `open()` are now `ValueError`. Frozen interface declares `artifact: pathlib.Path` and all existing callers/tests use `Path`; CLI builds `Path(args.artifact)`, so no CLI regression. Acceptable.
- **Additive tests only:** `test_invalid_board_entries_with_valid_submission` (tasks `None`/`[]`/entry-`None`, attempt `True`/`None`, owner `42`/`""`, state `[]`) and nesting CLI test exercise existing branches — no weakening of stale/closed/mismatch, digest, attestation, size-bound, or read-only invariants observed. No implementation change besides the guard.
- **Recursion explicitly not fixed:** no change to `_load_json` except handling; lead states 1100/3900 nesting did not reproduce on this Python. New `test_cli_rejects_excessive_nesting_without_traceback` therefore documents current-interpreter behavior, not a code fix — version-sensitive but honestly scoped; not raised as defect.

### Findings: none

No new behavior / stale-attempt / CLI-validation defect in the delta. Guard message wording (“not a file descriptor”) is imprecise for `None`/`[]` but functional and concise — not filed.

### Engineering log

- **Actions Taken:** Read both candidate-02 files; diffed mentally against attempt-1 review (guard + 3 tests + `patch` import only).
- **Issues & Friction:** Read-only so RED/green and hash claims unverifiable; nesting-test portability unconfirmable without execution.
- **Decisions & Rationale:** Accepted `Path`-only strictening as contract-conformant; did not demand recursion fix per lead’s explicit non-claim.
- **Solutions Applied:** None.
- **Insights:** Fix is minimal and well-targeted; test-with-`open`-patched is the right discriminator for the fd-0 risk.

