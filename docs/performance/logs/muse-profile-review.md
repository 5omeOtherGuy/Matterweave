## Review — frozen slice only (read-only, no execution)

**Scope:** `profile-candidate-01/tools/performance/validate_frame_profile.py`, `test_validate_frame_profile.py` vs `apps/explorer/src/metrics.rs` + `docs/performance/measurement-v2.md` + `muse-profile-a2-brief.md`. Worker-reported 17 focused / 113 total + host-90 / device-1000+581 pass **not re-run**.

**Verified (no finding):**
- Exact types: `COLUMNS` 37 + header magic match `metrics.rs:COLUMNS`/`FrameRow::write`; IDs exact-int via regex+`int()` (no `float`), limits u64/u32, `gpu_prev_shadows` blank/0/1, durations finite≥0, missing stays `None` — matches `write_ms`/`write_id`/`write_count` empty-is-never-zero.
- Identity joins: `(epoch,id)` pairs, per-epoch submission monotonicity, completion both-present-or-absent, future-epoch/duplicate/orphan rejection, no CPU→GPU join except via submission/completion pairs. `unmatched = submitted−completed` correctly lumps missing-prior-completion and trailing-unfinished as unknown without fabricating joins.
- Bounded streaming: `readline(16KiB+1)` + total 128MiB + 240k rows, no stat-then-read, `raw_sha256` over consumed bytes; CLI catches `ValueError`/`OSError`, concise stderr, no traceback, no mutation.
- Malformed rejection: bad schema/header/width/empty/overlong-line/row-cap/byte-cap all `ValueError`; `csv.reader` per-line safe for this comma/int-only dialect.

### Finding 1 (test covers wrong reason; same-epoch branch unproven)
- **File:line:** `test_validate_frame_profile.py:268-275` vs `validate_frame_profile.py:295-297` / `:322-325`.
- **Trigger:** `late` case uses `submitted_gpu_frame_id="1"` on both rows; row 2 fails at per-epoch monotonicity (`1 <= max 1`), never reaches same-epoch `completed_id >= submitted_id` check.
- **Consequence:** Test passes (any `ValueError`) but does not exercise the intended same-epoch-ordering rule; no other test isolates it (orphan/duplicate cases fail earlier).
- **Evidence:** Validator checks `submitted_max` (line 295) before completion ordering (line 322); test rows reuse id 1 in epoch 1.
- **Uncertainty:** Low; static path analysis only, no run.
- **Check:** Change row 2 to `submitted="2", completed="2"/epoch="1"` (with row 1 `submitted="1"`) so monotonicity passes and ordering check is the fail point; assert message contains `not before submission`.

## Engineering log
- **Actions Taken:** Read validator, test file, `metrics.rs`, `measurement-v2.md`, A2 brief; `find`/`grep` for line refs and candidate layout (`ls` shows candidate has only `docs/`,`tools/`).
- **Issues & Friction:** None blocking; `REPO_ROOT=parents[2]` path resolves to candidate root in frozen layout vs worktree canonical location — not filed since worker reports pass in canonical placement and I did not execute.
- **Decisions & Rationale:** Filed only the one isolatable intended-reason defect; did not invent joins, performance claims, or broad-engine findings.
- **Solutions Applied:** None (read-only leaf).
- **Insights:** Validator is contract-faithful on types/joins/missingness/bounds; test suite's main gap is branch-isolation discipline, not coverage breadth.

