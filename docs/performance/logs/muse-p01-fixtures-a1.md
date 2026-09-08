# P01 fixtures A1 — first deterministic authoritative-world fixture/replay slice

Bounded worker leaf. Owns ONLY `crates/matterweave-core/tests/performance_replay.rs`,
`crates/matterweave-core/tests/fixtures/performance_replay_v1.json`. No production
source, dependency, or global docs edits. Base `7556204`. No orchestration, commits,
branch changes, detached processes, credentials, or phone involvement.

## Actions Taken
- Read `crates/matterweave-core/src/{lib,streaming,persistence}.rs`, existing
  `tests/{api,world,streaming}.rs`, and `crates/matterweave-core/Cargo.toml` before writing.
- Wrote versioned JSON tape (`fixture_version: 1`, `generator_version: 1`,
  `seed: 20260907`, scenario `p01-baseline-a1`): 6 baseline probes + 11 ordered
  `stream`/`set` steps with exact coordinates, materials, and `expect_applied`,
  plus pinned `expected` cells/stats. Loaded via `include_str!`.
- Wrote test-only replay runner on real public APIs (`generate`, `enable_streaming`,
  `stream_around`, `set`/`get`, `save`/`load`, `stats`, `chunk_revision`,
  `contains_stream_cell`): (1) baseline determinism vs pinned probes/stats,
  (2) travel 0→+48→−48→0 across ± chunk boundaries with eviction + fast reversal,
  (3) double-replay exact save-bytes equality and save/reload/resave byte stability.
- TDD RED: stub validator (shape-only parse) + 4 broken-variant tests failed as
  required; probe test printed implementation-authoritative values used for pinning.
- TDD GREEN: strict validator (`deny_unknown_fields`, version checks against crate
  `GENERATOR_VERSION`, finite positions, in-domain cells, 1..=128 steps, non-empty
  expected state). All 10 tests pass; `rustfmt` check clean.
- Serialized Cargo use: `ps` showed no active cargo/rustc, so ran the locked
  core-only focused suite with `CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/fixture-target`
  (left in place; lead owns cleanup once workers stop).

## Issues & Friction
- RED-run compile error (`Option<Value>` closure return in `broken_variant`); fixed
  with a block closure. No production impact.
- `cargo fmt --check` flagged 3 hunks (import order, chain width, long asserts);
  fixed with `rustfmt --edition 2021` on the owned file, re-ran suite GREEN.
- Full-repo `git status` shows unrelated pre-existing modifications (engine
  instrumentation, tools, `docs/STATUS.md`, `Cargo.lock`); none touched by this leaf.

## Decisions & Rationale
- Materials chosen to differ from probed pre-edit values so every `expect_applied`
  is meaningful (e.g. `[0,2,0]`/`[-1,2,0]` legacy `2`→`7`; `[-17,2,-17]` air→`5`;
  outer-terrain `[48,2,0]` gen `1`→`9`, `[-48,2,0]` gen `1`→`4`).
- Pinned source-authoritative facts only: baseline `32` chunks / `42840` solid,
  final `98` / `131992` / `34` overrides; no FPS, capacity, or custom-hash claims
  (stdlib + `serde_json` already present; no new dependencies).
- Evicted chunks read as air with `chunk_revision == None`; persistence is proven
  by override restore on reversal, and save-bytes equality across two fresh replays
  proves determinism beyond revision comparison.
- Left to later slices (not implemented, not claimed): 64-body/physics, app input
  replay integration, camera/input/hardware captures. This is host regression proof,
  NOT phone replay support.

## Solutions Applied
- Tape keeps one deliberate no-op per class: same-material set, evicted-chunk set
  (rejected, reads air), out-of-domain set — all assert revision is unchanged.
- Independent Euclidean-boundary guard: `-17→chunk -2`, `-16→chunk -1`,
  `15→chunk 0`, `16→chunk 1` asserted on a fresh world outside the fixture path.
- Fixture stays small (tape + pinned expectations only); no world bytes copied.

## Insights
- `stream_around` centers are `floor(pos)/16 div_euclid` clamped to ±16, so
  ±48 lands on centers ±3 with windows that fully evict the opposite side's edited
  chunks — ideal for a compact eviction/reversal proof.
- `stored_overrides` grew 32→34 exactly for the two new outer-terrain chunks,
  a sharp, cheap assertion that edits landed where intended.

## Verification (actual)
- RED: `cargo test --locked -p matterweave-core --test performance_replay` →
  2 passed, 4 failed (`unsupported_fixture_version/generator_version`, `unknown_op`,
  `malformed_tape` empty-steps variant). `missing_expected_state` and probe passed.
- GREEN: same command → **10 passed, 0 failed** (also re-passed after `rustfmt`).
- `cargo fmt --check` on owned Rust file: clean. Fixture parses as JSON: OK.
- NOT RUN by worker: independent reviews, integration suites beyond the focused
  test, device/phone runs. Full-workspace builds/tests not started (Opus owns the
  heavy build lane).
