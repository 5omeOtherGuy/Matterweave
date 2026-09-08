# Performance campaign execution record

The sole live assignment authority is [board.json](board.json), written only by the
lead. The v0.3 SQLite board remains closed history; it is not used for this run.
This records execution of [P00–P07](../PERFORMANCE_TASKS.md), not a replacement plan.

## Patch handoff contract v1

Workers do not commit. Each attempt has one owner, exact base, disjoint literal
owned paths, a Pi run/session directory, an engineering log and lead-owned checks.
The lead checks ownership before dispatch; at most three workers run concurrently.
No worker may detach jobs, spawn descendants, alter the board or access the phone.
Startup briefs stay under 6 KiB; status under 2 KiB; messages under 1 KiB, at most
four/8 KiB per inbox. Large evidence is referenced, not copied into the board.

After a worker finishes or is cancelled, the lead verifies its runner and children
are inactive, preserves tracked **and untracked** changed files, and freezes their
bytes into an artifact with a SHA-256. Preserve partial work before replacing an
attempt. Record the supervision evidence separately; `verified_inactive` is a
manual attestation, not an operating-system liveness detector. Replacements increase
the attempt and use a new owner. Never accept a late old-attempt result.

`tools/performance/check_handoff.py` is a read-only admission guard, not another
board, automatic review, authentication, process supervisor or filesystem sandbox.
It checks current run/task/attempt/owner, eligible state, inactive attestation and
frozen artifact hash. A passing submission still requires independent Muse/Gemini
review, then Astra review and relevant integration checks. Submission is not acceptance.
The lead checks reviewed artifact hashes again before applying or accepting changes.

States: running, submitted, review-needed, accepted, rejected, blocked. Closed
attempt evidence stays in logs; keep only useful current assignments on the board.
The JSON board and raw execution artifacts must not disagree about ownership.

## Current evidence

- [P00 preflight and lead engineering log](p00.md)
- [Model outcomes](models.md)
- [First unplugged app baseline](../evidence/2026-09-08-performance-baseline.md)
- Original baseline source: `2ce6ce3157e791ab6dd49ed00cd865ee5c1cacc6` (remote tag
  `v0.3.0` verified; this checkout initially had no local release tag).
- Campaign starting source: `755620472e8457d8e55760165e759b6d8013c6db`, from
  `codex/performance-orchestration-plan`; own branch `codex/performance-p00`.
- The required [alien fungal wetland](../SHOWCASE.md) is not implemented yet.
