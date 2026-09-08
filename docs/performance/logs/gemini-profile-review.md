### Independent Read-Only Review: Frozen Profile Validator Slice

Reviewed frozen artifacts:
- `/mnt/bench/matterweave-dev/performance/run-01/profile-candidate-01/tools/performance/validate_frame_profile.py`
- `/mnt/bench/matterweave-dev/performance/run-01/profile-candidate-01/tools/performance/test_validate_frame_profile.py`

Contract source and assignment:
- `apps/explorer/src/metrics.rs` (`SCHEMA_VERSION = 2`, `COLUMNS`, `write_*` rules)
- `docs/performance/measurement-v2.md`
- `performance/run-01/muse-profile-a2-brief.md`

*Execution was NOT run (strictly read-only inspection).*

---

### Concrete Findings

#### Finding 1
- **file:line**: `/mnt/bench/matterweave-dev/performance/run-01/profile-candidate-01/tools/performance/test_validate_frame_profile.py:271`
- **trigger**: In `test_completion_identity_rules`, the `late` test case intended to verify the rule `"Same-epoch completion must precede this row's submission"` constructs row 2 with `make_row(draw_attempt_id="2", presented_count="2", submitted_gpu_frame_id="1", completed_gpu_frame_id="1", completed_gpu_renderer_epoch="1", ...)`. Because row 1 was also defined with `submitted_gpu_frame_id="1"`, row 2 repeats the same submission ID in epoch 1.
- **consequence**: Row 2 fails prematurely at `validate_frame_profile.py:279` (`submitted_id <= submitted_max.get(epoch, -1)`) with `ValueError: line 4: submitted_gpu_frame_id 1 not increasing in epoch 1`. It never reaches the intended check at lines 316–318 (`same-epoch completion {completed_id} is not before submission {submitted_id}`). The test falsely passes `pytest.raises(ValueError)` for the wrong reason, leaving the same-epoch completion precedence invariant (`completed_id >= submitted_id`) completely unexercised by the test suite.
- **evidence**: In `test_validate_frame_profile.py:269-272`, row 1 has `submitted_gpu_frame_id="1"` and row 2 has `submitted_gpu_frame_id="1"`. In `validate_frame_profile.py:279-281`, `1 <= 1` unconditionally raises before line 316.
- **uncertainty**: None; static evaluation of the row sequence confirms line 279 aborts before line 316 executes.
- **check**: Update row 2 in `test_validate_frame_profile.py:271` to set `submitted_gpu_frame_id="2"` and `completed_gpu_frame_id="2"`, and assert `match="is not before submission"` to pin the exact failure reason.

---

#### Finding 2
- **file:line**: `/mnt/bench/matterweave-dev/performance/run-01/profile-candidate-01/tools/performance/validate_frame_profile.py:233` (also `:214`, `:358`)
- **trigger**: Malformed CSV input containing an unclosed double-quote (`"`) or NUL byte (`\0`) causes `next(csv.reader([text]))` to raise `_csv.Error` (e.g. `unexpected end of data` or `line contains NULL byte`).
- **consequence**: `validate_profile()` lets `csv.Error` escape instead of converting it to `ValueError`, violating the API contract (`validate_profile(path) -> summary dict or raise ValueError`). In CLI mode (`main()`), line 358 catches only `(ValueError, OSError)`; because `csv.Error` inherits directly from `Exception` in Python standard library (`issubclass(csv.Error, (ValueError, OSError))` is `False`), the exception is unhandled, printing a Python traceback to stderr. This violates the CLI contract: *"CLI prints JSON/nonzero concise error without traceback; no mutation. Reject wrong schema, header, widths, empty capture, malformed CSV."*
- **evidence**: Lines 214 and 233 invoke `csv.reader` without a `try ... except csv.Error: _fail(...)` block. Line 358 catches only `(ValueError, OSError)`.
- **uncertainty**: Native Matterweave instrumentation in `metrics.rs` writes unquoted numbers and identifiers and does not emit quotes or NUL bytes, so this occurs only on corrupt or malformed inputs.
- **check**: Wrap `csv.reader` row parsing in `try ... except csv.Error as error: _fail(f"line {lineno}: malformed CSV: {error}")`, or add `csv.Error` to the caught types in `validate_profile` and `main()`.

---

### Verification Assessment

1. **Exact Types & Identity Joins**:
   - `draw_attempt_id` and `renderer_epoch` are strictly checked as positive decimal integers with `u64` limit (`2^64 - 1`), preserving exact integer identities without float conversion (safe for values $> 2^{53}$).
   - `submitted_gpu_frame_id` strictly increases per epoch; counters restart across epochs; pairs `(renderer_epoch, submitted_gpu_frame_id)` correctly isolate submissions across renderer recreations.
   - `completed_gpu_frame_id` and `completed_gpu_renderer_epoch` require both present or both absent, reject future epochs, reject duplicate completions, and reject completions not previously submitted (`orphan completion`).
   - Line 282 (`if pair in completed:`) is unreachable defense-in-depth because monotonic submission checks and orphan completion checks ensure no pair can be in `completed` prior to its submission.

2. **Missingness vs Zero**:
   - Empty cells are parsed as `None` (not `0` or `0.0`), strictly counted in `missing_cells`.
   - Durations are validated as non-negative finite floats (`[0-9]+(\.[0-9]+)?...`); negative, NaN, and infinity values are rejected.
   - `gpu_prev_*` columns are strictly rejected if populated without completion identity.
   - `shadow_caster_meshes` is strictly rejected if populated on an attempt without a GPU submission.
   - Partial body partitions and partial save counters are permitted without fabricating missing components.

3. **Bounded Streaming**:
   - `_BoundedReader` enforces streaming readline with `MAX_LINE_BYTES = 16 KiB`, `MAX_TOTAL_BYTES = 128 MiB`, and `MAX_ROWS = 240,000`.
   - Streaming hash (`reader.digest`) runs over raw bytes consumed without stat-before-read or double-reading.
   - EOF without trailing newline on the final row is accepted cleanly.

4. **Trailing Submissions vs Missing Prior Completions (No Fabricated Joins)**:
   - Trailing submissions (such as the final frame of an epoch or capture where in-flight GPU completion had not returned before capture ended) and missing prior completions are both tracked in `submitted - completed`.
   - The validator correctly counts these in `unmatched_completions` without attempting to invent joins to unrelated rows.
   - On the real capture data, `host-smoke-02` (unmatched: 2) and `device` 581-row (unmatched: 3) correspond to exactly one trailing submission per epoch; the 1000-row device capture (unmatched: 1) corresponds to the single trailing submission at capture termination.

---

### Concise Engineering Log

- **Actions Taken**:
  - Read lead brief `muse-profile-a2-brief.md`, Rust metrics implementation `metrics.rs`, capture specification `docs/performance/measurement-v2.md`, and worker log `muse-profile-a2.md`.
  - Read candidate validator `validate_frame_profile.py` and test suite `test_validate_frame_profile.py`.
  - Inspected real capture samples (`host-smoke-02` 90 rows, device 1000 rows and 581 rows) in place without mutation or execution.
  - Formally analyzed line-by-line contract compliance: bounded I/O, regexes, type conversions, state tracking, and exception propagation.
  - Verified test case mechanics against the validator's branch execution paths.

- **Issues & Friction**:
  - `late` test case in `test_validate_frame_profile.py:271` tripped the submission monotonicity guard on line 279 rather than the completion precedence check on line 316, obscuring test intent.
  - `csv.Error` exception class discrepancy: standard Python `csv.reader` errors do not inherit from `ValueError` or `OSError`, leaving a narrow hole in malformed CSV rejection where tracebacks leak.

- **Decisions & Rationale**:
  - Retained exactly 2 concrete findings (below the $\le 3$ limit) with file:line, trigger, consequence, evidence, uncertainty, and check.
  - Avoided flagging judgment calls (e.g. counting head gap in `missing_attempts` when starting at draw ID $> 1$, or accepting trailing lines without `\n`) as defects, as they are documented decisions matching lead specification.
  - Strictly respected read-only leaf constraint: executed no tests/shell commands, modified no files, and invoked no subagents.

- **Solutions Applied**:
  - Identified precise test correction for Finding 1 (`submitted_gpu_frame_id="2"`, `completed_gpu_frame_id="2"`).
  - Specified clean exception wrapping for Finding 2 to ensure `validate_profile()` and `main()` fulfill the zero-traceback contract on all malformed CSV inputs.

- **Insights**:
  - Relying on bare `pytest.raises(ValueError)` without checking the exception message or regex match (`pytest.raises(ValueError, match=...)`) allows tests to pass for unintended upstream validation errors.
  - In asynchronous GPU capture pipelines, trailing submissions at epoch or process boundaries naturally outnumber genuine mid-stream completion drops; tracking them as `submitted - completed` without fabricating joins provides an accurate, verifiable invariant.
