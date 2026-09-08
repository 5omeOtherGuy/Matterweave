## Correction Review — frozen slice only (read-only, no run)

Scope: `performance/run-01/profile-candidate-02/tools/performance/validate_frame_profile.py` + `test_validate_frame_profile.py`, `correction.patch` vs `candidate01`, contract `apps/explorer/src/metrics.rs` + `docs/performance/measurement-v2.md`, corrections `run-01/muse-profile-a3-brief.md`. Engine/P03 not reviewed. Execution NOT RUN by me; lead evidence cited as-is.

### Actions
- Read frozen validator + tests in full.
- Read `correction.patch` (validator + test delta vs candidate01).
- Read brief items 1–7 and checked each against delta.
- Read contract `metrics.rs: COLUMNS/SCHEMA_VERSION/write rules`, `measurement-v2.md` identities/timings/counters.

### Issues checked (brief → delta)
1. pytest→unittest: test now `unittest.TestCase`, no `pytest` import, synthetic fixtures explicit, `discover -s tools/performance` documented. Contract pin retained via regex on `metrics.rs`.
2. Late-completion reason: second row `submitted=2/completed=2` after `submitted=1`, asserts `not before submission` — avoids earlier duplicate-submission mask. Patch order: same-epoch causal check before duplicate check. Correct.
3. Strict CSV: new `_parse_csv_line(..., strict=True)` translates `csv.Error→ValueError`; header, rows share it. Tests: unclosed final `"` + embedded `1\r2` → `ValueError`, CLI concise `error:` no traceback. Matches lead repro (Gemini NUL theory rejected).
4. 16 KiB incl. terminators: `readline(MAX+1)`, `len(chunk)>MAX` on raw bytes before `CRLF/LF` strip, hash covers raw. Test at/over bounds for `b""`, `b"\n"`, `b"\r\n"`. Correct.
5. Semantics split: `unmatched_completions` = observed completion with missing submission row only if `missing_attempts>0`, else orphan `ValueError`; `uncompleted_submissions=len(submitted-completed)` (unfinished/unobserved, not GPU failure). Gap fixture `1→3 submit3/complete2`: `missing1/unmatched1/uncompleted2`. Single presented row: `0/1`. No fabricated join.
6. Missingness: `missing_cells: dict` over 33 optional (all except `draw_attempt_id/renderer_epoch/presented_count/result`) + `missing_cells_total`. Per-column test expects 30/30.
7. Descriptors/IO: `isinstance(bool,int)` + `os.fspath` reject before `open`; mocked-`open` not-called test for `0/1/False/True/None/["x"]`; `readline OSError→ValueError`; `open OSError→ValueError`; missing file → `ValueError`.
- Guards preserved: future epoch, same-row `>=`, duplicate pair, draw strictly increasing, epoch non-decreasing, presented-delta `increment≤delta≤increment+gap`, submission per-epoch increase, `gpu_prev_*` gated, body partition, `save_failures≤attempts`. Both identity domains exact `u64` (`U64_MAX`), counts `u32`; `>2^53` + overflow tests present.

### Findings (concrete-error hunt, ≤3) — none valid
- Candidate A — `validator.py:_validate_rows` head-gap `draw_id-0-1`: single large-`draw_id` row counts huge `missing_attempts`. Trigger: first-row gap from zero. Consequence: surprising but unchanged from candidate01, contract silent on head-gap, brief did not order change. Evidence: code `prev_draw=0`; test deliberately drops old head-gap assert. Uncertainty: none on correctness of delta. Check: leave as-is; out-of-scope. **Rejected — not a defect.**
- Candidate B — `validator.py:_BoundedReader` `readline(MAX+1)` truncation: very long line returns `MAX+1` bytes without terminator, still `>MAX` → reject without unbounded read. Trigger: `b"9"*20000`. Consequence: none, bounded. Evidence: test `long_physical_line_rejected` + byte-bound at/over. Check: code path inspected. **Rejected — not a defect.**

Blocking defects: **0**.

### Decisions / Solutions
- Accept delta as faithful to all 7 brief items; no scope bleed into engine/P03 observed.
- Do not conflate `uncompleted_submissions` (submitted-minus-completed) with `unmatched_completions` (completed-without-observed-submission in gap).

### Insights
- Lead runtime evidence (130 unittest PASS; raw-cap + IO RED/GREEN after initial `NameError` fixture fix) is the execution record; I did not re-run per frozen/read-only constraint.
- Remaining gap (not a correction defect): head-gap semantics and CLI device/host counts remain lead-verified, not re-proven here. Stop at frozen slice.

