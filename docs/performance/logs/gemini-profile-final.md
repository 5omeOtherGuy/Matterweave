### Independent Correction Review: Frozen Profile Candidate 02

**Target Slice:**
- `/mnt/bench/matterweave-dev/performance/run-01/profile-candidate-02/tools/performance/validate_frame_profile.py`
- `/mnt/bench/matterweave-dev/performance/run-01/profile-candidate-02/tools/performance/test_validate_frame_profile.py`
- `correction.patch` against `profile-candidate-01`
- Reference Contracts: `apps/explorer/src/metrics.rs`, `docs/performance/measurement-v2.md`, `run-01/muse-profile-a3-brief.md`
- Constraints: Read-only leaf review; execution NOT RUN by this agent; no edits/shell/agents/phone; engine/P03 excluded.

---

### Verification Against Review Brief & Contract Specifications

1. **Test Runner Migration (`unittest` not `pytest`):**
   - Candidate 02 completely replaced pytest fixtures and decorators with `unittest.TestCase` and `self.subTest`.
   - CI discovery pattern `python3 -m unittest discover -s tools/performance -p 'test_*.py'` passes 130 tests (`profile-lead-green-corrected.log`).
   - All fixtures in `test_validate_frame_profile.py` are strictly synthetic; private real device captures are removed from CI test code.

2. **Strict CSV & Normalization / IO Failure Handling:**
   - `_parse_csv_line` parses each physical line via `csv.reader([text], strict=True)`.
   - `csv.Error` (e.g. unclosed final quotes, embedded CRs) is caught and translated to `ValueError` (`line {lineno}: malformed CSV: {error}`).
   - Embedded CRs and unclosed quotes are verified rejected concisely without stack trace leaks in `TestMalformed` and `TestCLI`.
   - `_BoundedReader.__init__` and `__iter__` catch `OSError` on open/readline and translate to `ValueError`. Initial test fixture `NameError` (`Path` vs `patch`) was corrected prior to capturing valid IO RED (`OSError: synthetic EIO` in `profile-read-error-red-corrected.log`) and subsequent GREEN (`profile-lead-green-corrected.log`).

3. **Physical Line Cap (Raw 16 KiB INCLUDES Terminators):**
   - Reader reads `MAX_LINE_BYTES + 1` bytes and enforces `len(chunk) > MAX_LINE_BYTES` before stripping line terminators (`\r\n` or `\n`).
   - Lines with 16,384 bytes including terminator are accepted; lines with 16,385 bytes are rejected for LF, CRLF, and un-terminated EOF chunks (`TestMalformed.test_physical_line_byte_bounds`).
   - `raw_sha256` continues to digest all raw bytes as read.

4. **Descriptor Rejection Prior to Open:**
   - `validate_profile` explicitly checks `if isinstance(path, (bool, int)): _fail(...)` before calling `os.fspath` or `open`.
   - Rejects descriptors `0`, `1`, booleans `False`, `True`, and foreign types (`None`, `["x"]`) with `ValueError`.
   - `TestMalformed.test_descriptors_rejected_before_open` verifies mocked `builtins.open` is never called.

5. **Missingness Accounting (`missing_cells` per Column):**
   - `missing` is now a dictionary tracking missing counts across all 33 optional columns (`OPTIONAL_COLUMNS`).
   - Summary dictionary returns `"missing_cells": missing` and `"missing_cells_total": sum(missing.values())`.

6. **Completion vs. Submission Accounting (Deliberate Separation):**
   - **`unmatched_completions`:** Counts observed `(completed_gpu_renderer_epoch, completed_gpu_frame_id)` completion pairs that lack an observed submission row. Only permitted when prior draw attempts are missing (`missing_attempts > 0`); with zero missing attempts, orphan completions are rejected with `ValueError`.
   - **`uncompleted_submissions`:** Evaluated as `len(submitted - completed)`. Counts submitted GPU frames never observed as completed (unfinished or unobserved work, not a GPU failure).
   - Prevents conflation of `unmatched_completions` with `len(submitted - completed)`. Single presented row correctly yields `unmatched_completions: 0`, `uncompleted_submissions: 1`. Synthetic gap sequence (draw 1 submits 1; draw 3 submits 3 and completes 2) correctly yields `missing_attempts: 1`, `unmatched_completions: 1`, `uncompleted_submissions: 2`. No join is fabricated.

7. **Ordering, Causal, and ID Guards:**
   - Future epochs (`completed_epoch > epoch`), same-row completions at or after same-epoch submissions (`completed_id >= submitted_id`), duplicate completions, duplicate submissions within an epoch, and time-reversed submissions (`submitted` reusing an already `completed` pair) are strictly rejected.
   - Second-row `submitted=2, completed=2` after `submitted=1` asserts the exact `'not before submission'` reason (`TestIdentity.test_late_completion_reports_not_before_submission`).
   - Non-increasing draw attempt IDs and decreasing renderer epochs are rejected; gaps remain unknown.
   - Both identity domains (`draw_attempt_id` and GPU frame/epoch identities) are parsed as exact `u64` integers (capped at `2**64 - 1`, rejecting floats, signs, and negatives).

---

### Audit of Findings

Evaluation of potential candidate defects in the delta:

1. **Candidate Finding 1: Cumulative vs. per-epoch missing attempts check for gap completions**
   - **Line:** `validate_frame_profile.py:366`
   - **Trigger:** A capture with missing draw attempts in epoch 1, followed by epoch 2 observing a completion that was never submitted.
   - **Consequence:** The unobserved completion in epoch 2 increments `unmatched_completions` instead of raising an orphan completion `ValueError`.
   - **Evidence:** `validate_frame_profile.py:366` evaluates `if missing_attempts == 0:`.
   - **Uncertainty:** `run-01/muse-profile-a3-brief.md` item 5 states: *"With no prior missing draw attempts, unknown completion remains invalid... never fabricate a join."* The global missing attempt counter is the specified contract metric; per-epoch gap tracking was intentionally not introduced to avoid speculative counter reconstruction across gaps.
   - **Check:** **None valid** (adheres directly to specification).

2. **Candidate Finding 2: Unbounded gap completion ID on rows without submission**
   - **Line:** `validate_frame_profile.py:359–369`
   - **Trigger:** A draw gap followed by a row without a submission (`submitted_id is None`) observing an arbitrarily large completion ID.
   - **Consequence:** The completion is accepted as `unmatched_completions` without bounding `completed_id` against the count of missing draw attempts.
   - **Evidence:** Causal check `completed_id >= submitted_id` is only guarded when `submitted_id is not None`.
   - **Uncertainty:** Engine draws in gaps may have produced retries or multiple internal state changes; fabricating an artificial ceiling on unobserved submission IDs violates the requirement that *"gaps remain unknown."*
   - **Check:** **None valid** (consistent with identity-only scope without fabricated joins).

3. **Candidate Finding 3: Redundant import in test method**
   - **Line:** `test_validate_frame_profile.py:538`
   - **Trigger:** Execution of `test_read_failure_is_value_error`.
   - **Consequence:** Re-imports `from unittest.mock import patch` when already imported at line 20.
   - **Evidence:** Redundant local statement.
   - **Uncertainty:** Harmless Python idiom; zero semantic impact.
   - **Check:** **None valid** (not a functional error).

**Conclusion:** **None valid.** The frozen implementation and test suite in `profile-candidate-02` satisfy all requirements and contract bounds without concrete errors.

---

### Compact Engineering Log

- **Actions:**
  - Audited frozen files in `profile-candidate-02` and verified git diff in `correction.patch`.
  - Reconciled column list and rules against `apps/explorer/src/metrics.rs` and `docs/performance/measurement-v2.md`.
  - Audited recorded test evidence in `profile-lead-red.log`, `profile-read-error-red-corrected.log`, and `profile-lead-green-corrected.log`.
  - Checked strict CSV parsing, 16 KiB raw line cap (with terminators), descriptor pre-check, per-column missingness, and completion/submission accounting.
  - Formulated candidate findings and verified their invalidity against written specifications.
  - Confirmed execution was NOT run in this session.

- **Issues:**
  - Historical conflation between `unmatched_completions` (unobserved submissions for observed completions) and `uncompleted_submissions` (unobserved completions for observed submissions).
  - Pytest unpinned runner incompatible with CI standard `unittest` discovery.
  - Prior line-cap bypass where LF was stripped prior to measuring raw line length.
  - Previous NameError in test suite before capturing valid IO runtime RED.

- **Decisions:**
  - Enforce `unmatched_completions` strictly for completions occurring when `missing_attempts > 0` and unobserved in `submitted`.
  - Provide both `missing_cells` dictionary and `missing_cells_total` integer.
  - Reject all integer/boolean descriptors before touching `open()`.
  - Keep gaps unknown without synthesizing missing frame tracking.

- **Solutions:**
  - Frozen `profile-candidate-02` cleanly addresses all review directives from `muse-profile-a3-brief.md`.
  - Synthetic test suite in standard `unittest` provides full contract coverage across 130 passing tests.

- **Insights:**
  - Separating `unmatched_completions` from `uncompleted_submissions` eliminates false claims of GPU failure while preserving causal completion verification.
  - Checking `len(chunk) > MAX_LINE_BYTES` before line-ending stripping prevents terminator-leak overruns under both LF and CRLF encodings.
