# Instrumented engine source coverage

Frozen source `b1f6c67`, Rust1.96.0, host x86_64 Linux. The four engine crates'
unit/integration tests passed, followed by instrumented Vulkan cache, instancing,
shadow-cache and indirect examples under llvmpipe with validation and no errors.
These are execution measurements, not engine-completion or Android-quality percentages.

| Engine crate | Executed LCOV lines / mapped lines | Coverage |
| --- | --- | --- |
| core | 907 / 935 | 97.01% |
| detail | 3193 / 3329 | 95.91% |
| physics | 1347 / 1407 | 95.74% |
| render | 2901 / 3066 | 94.62% |

## Method and qualification

Build with `RUSTFLAGS="-C instrument-coverage"`, a dedicated target directory,
`CARGO_BUILD_JOBS=1` and `LLVM_PROFILE_FILE` containing `%m-%p`. Run
`cargo test --locked -p matterweave-core -p matterweave-detail -p matterweave-physics -p matterweave-render --lib --tests`.
Then run the render examples `cache_smoke`, `instancing_smoke`,
`shadow_cache_smoke`, `indirect_smoke` with the same instrumentation. Merge profiles
using Rust1.96's `llvm-profdata merge -sparse`.

A single `llvm-cov` invocation containing all27 binaries warned about180 hash
mismatches. Astra's bounded audit identified30 unique symbols,16 from the project,
all with missing hash-zero records. Reversing object order changed five project
function counts, including `SceneVersion::eq` and renderer getters. That combined
aggregate is unsuitable as an exact coverage result and was not accepted.

The lead instead exported each binary separately using `llvm-cov export
-format=lcov -instr-profile=combined.profdata`, then formed a union keyed by
source filename and mapped line. A line is executed only if at least one individual
export reports a positive hit. This union is independent of input order and does
not add counts from duplicate copies. Per-object hash-zero diagnostics remain
recorded; this is a scoped LCOV line-union result, not a clean function/region or
branch-coverage certification. Do not substitute the earlier combined percentages.

Scope is `crates/*/src`, excluding separately named `*_tests.rs` files. Inline
`cfg(test)` blocks and source fixtures remain included. Dependencies, vendor,
examples, separate integration tests, app code and GPU shaders are excluded from
the denominator. Shader behavior is exercised by the native examples, but shader
coverage is not instrumented. Later queue/numeric/native-adapter commits are not
part of this frozen measurement; their ordinary regression checks are separate.

## Reproducible evidence and engineering log

Raw outputs,27 per-object LCOV files, exact object manifests, profile data and the
lead's `union_lines.py` are under
`/mnt/bench/matterweave-dev/performance/engine-02/coverage`. The small
[summary](../evidence/2026-09-08-engine-coverage.json) records the source and method.
The temporary generated build target can be rebuilt from the frozen revision;
no desktop timing is presented as a mobile performance result.

**Actions:** lead ran instrumented tests and four native examples; Astra examined
profile/object mismatches and reversed object order; lead reviewed the partial
artifacts and computed individual-object line unions.

**Issues:** `cargo llvm-cov` was unavailable, so installed matching LLVM tools were
used directly. Astra reached its360-second deadline before a final report. Its
partial diagnosis was independently inspected, not counted as a completed review.

**Decision:** retain counter warnings and scope explicitly. Do not infer uncovered
shader/Android/error paths are safe from a high host source-line percentage.
