# Instrumented engine source coverage

Frozen source `b1f6c67`, Rust 1.96.0, host x86_64 Linux.

## What problem this solves

Passing tests do not say which engine source lines the tests execute. This record
measures line coverage for the four engine crates under instrumentation. It is an
execution measurement, not an engine-completion or Android-quality percentage.

## How it works

1. Build with `RUSTFLAGS="-C instrument-coverage"`, a dedicated target directory,
   `CARGO_BUILD_JOBS=1` and `LLVM_PROFILE_FILE` containing `%m-%p`.
2. Run
   `cargo test --locked -p matterweave-core -p matterweave-detail -p matterweave-physics -p matterweave-render --lib --tests`,
   then the render examples `cache_smoke`, `instancing_smoke`, `shadow_cache_smoke`
   and `indirect_smoke` under the same instrumentation. The four engine crates'
   unit/integration tests passed, and all four examples ran under llvmpipe with
   validation and no errors.
3. Merge profiles with Rust 1.96's `llvm-profdata merge -sparse`.

A single `llvm-cov` invocation over all 27 binaries was rejected: it warned about
180 hash mismatches. Astra's bounded audit identified 30 unique symbols, 16 from
the project, all with missing hash-zero records; reversing object order changed
five project function counts, including `SceneVersion::eq` and renderer getters.
That combined aggregate is unsuitable as an exact coverage result and was not
accepted.

The reported result instead comes from exporting each binary separately with
`llvm-cov export -format=lcov -instr-profile=combined.profdata`, then forming a
union keyed by source filename and mapped line. A line counts as executed only if
at least one individual export reports a positive hit. The union is independent of
input order and does not add counts from duplicate copies. Per-object hash-zero
diagnostics remain recorded. This is a scoped LCOV line-union result, not a clean
function/region or branch-coverage certification. Do not substitute the earlier
combined percentages.

## What was verified

| Engine crate | Executed LCOV lines / mapped lines | Coverage |
| --- | --- | --- |
| core | 907 / 935 | 97.01% |
| detail | 3193 / 3329 | 95.91% |
| physics | 1347 / 1407 | 95.74% |
| render | 2901 / 3066 | 94.62% |

Scope is `crates/*/src`, excluding separately named `*_tests.rs` files. Inline
`cfg(test)` blocks and source fixtures remain included. Dependencies, vendor,
examples, separate integration tests, app code and GPU shaders are excluded from
the denominator. Shader behavior is exercised by the native examples, but shader
coverage is not instrumented. Later queue/numeric/native-adapter commits are not
part of this frozen measurement; their ordinary regression checks are separate.

Raw outputs, 27 per-object LCOV files, exact object manifests, profile data and
the lead's `union_lines.py` are under
`/mnt/bench/matterweave-dev/performance/engine-02/coverage`. The small
[summary](../evidence/2026-09-08-engine-coverage.json) records the source and
method. The temporary generated build target can be rebuilt from the frozen
revision.

## Limits and what is open

- `cargo llvm-cov` was unavailable, so installed matching LLVM tools were used
  directly.
- Counter warnings are retained deliberately, and scope is stated explicitly.
  Do not infer that uncovered shader, Android or error paths are safe from a high
  host source-line percentage.
- Astra reached its 360-second deadline before a final report. Its partial
  diagnosis was independently inspected, not counted as a completed review.
- The host ran the examples on llvmpipe; no desktop timing is presented as a
  mobile performance result.
