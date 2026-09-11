# Profile validator (P01 attempt 2) — implementation log

Leaf owned: `tools/performance/validate_frame_profile.py`,
`tools/performance/test_validate_frame_profile.py`. No agents, commits, jobs,
network, or capture copies. Captures read-only in place.

## What was built

Small strict CSV v2 format/identity validator, Python stdlib only
(`csv`, `hashlib`, `json`, `math`, `re`, `sys`). Source of truth:
`apps/explorer/src/metrics.rs` (`SCHEMA_VERSION`, `COLUMNS`, write rules) and
`docs/performance/measurement-v2.md`. Header verified exact:
`#matterweave-frame-capture schema_version=2`, 37 columns pinned in order,
regression-tested name-for-name against the Rust `COLUMNS` block.

API `validate_profile(path)` returns the summary dict or raises `ValueError`;
CLI prints JSON, exits nonzero with a concise stderr line and no traceback on
failure, never mutates input. Bounded streaming: binary `readline` with 16 KiB
physical-line, 128 MiB total, 240000-row caps; `csv` does parsing; missing
cells stay missing (counted, never zero-filled).

Enforced: positive u64 draw/epoch ids, u64 presented count, exact int ids
(>2^53 exact, u64/u32 overflow rejected), `presented|retry|out_of_memory`,
finite>=0 durations, `gpu_prev_shadows` blank/0/1, increasing draw ids,
nondecreasing epochs, presented-count delta rule (consecutive exact, gaps allow
only unknown intervening increments + current result, never backwards/overfull),
presented-requires-submission, no-submission ⇒ blank `shadow_caster_meshes`,
per-epoch increasing submissions, completion both/neither + not-future +
no-duplicate + must reference a previously submitted pair (cross-epoch ids
distinct), gpu_prev_* blank without completion, body partition and
save-failures⊆attempts checks, unmatched prior submissions counted as unknown.
No CPU/wall comparison (different clocks). Summary carries `schema_version`,
`row_count`, `renderer_epochs`, `missing_attempts`, `unmatched_completions`,
`missing_cells`, `raw_sha256`, `evidence='capture_format_and_identity_only'`.

Judgment calls (lead review): `missing_attempts` includes a head gap when the
first draw id > 1; `renderer_epochs` is a sorted list; `unmatched_completions`
counts submitted pairs never observed as completed; a same-epoch completion on
a no-submission row must still reference a previously submitted pair.

## Verification

- RED: new tests run against the missing module — collection failed / CLI test
  failed as expected (no validator file present).
- GREEN: `TMPDIR=…/run-01/scratch pytest
  tools/performance/test_validate_frame_profile.py -q` → **17 passed**.
  Covers: Rust-column regression, positive contract (incl. >2^53 ids, retries
  before/after submission, epoch reuse), missingness, overflow/invalid numerics,
  submission/completion identity, ordering/count deltas, malformed inputs and
  line/row/byte bounds (row and byte caps via monkeypatched limits for speed),
  CLI JSON + nonzero-no-traceback + no-mutation.
- Real captures, read-only (mtime/size asserted unchanged, sha256 of bytes
  matches `raw_sha256`): device `frame-profile-v2-1788829575868.csv` → VALID
  (1000 rows, epochs [1], missing 0, unmatched 1); device
  `frame-profile-v2-1788829637134.csv` → VALID (581 rows, epochs [1, 2, 3],
  missing 0, unmatched 3 = one trailing submission per epoch); host
  `frame-profile-v2-1788827612325.csv` → VALID (90 rows, epochs [1, 2],
  missing 0, unmatched 2).
- Not run: Android device execution / any performance measurement (out of
  scope; this tool makes no performance claims).

## Unresolved / for lead acceptance

- Independent review and acceptance are lead-owned; frozen paths are the two
  `tools/performance/` files above plus this log.
- Validator accepts a final line without trailing newline and counts head-gap
  ids as missing (see judgment calls); confirm these match acceptance intent.
