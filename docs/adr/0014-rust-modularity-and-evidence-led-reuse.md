# ADR-0014: Rust, modularity and evidence-led reuse

- Date: 2026-09-07
- Status: Accepted
- Basis: Explicit owner direction after discussing C++ and Rust implications.
- Requirements: R03, R04, R07, R11, R17, R18, R19, R20

## Context

The owner requires Rust wherever feasible without detriment to the project, plus modularity, maximum useful speed/efficiency and reuse of viable alternatives that meet strict criteria. New technology, tools and Rust hardware interfaces are authorized when necessary or significantly advantageous; Jolt is eligible only with major advantages.

This supersedes the C++ foundation proposal and the earlier build/reuse recommendation in ADR-0003/0004. Android, voxel worlds, graphics ambition, complex physics and multi-genre reuse remain governing goals.

## Decision

Use Rust by default for new engine systems, reusable libraries and tools wherever it is practical and not materially detrimental. Other languages remain eligible for shaders, required platform/vendor interfaces or a justified substantial advantage. Rust preference does not require rewriting suitable existing tools or proven dependencies just to change language. Record any material exception and keep its boundary narrow.

Evaluate existing solutions before substantial custom implementation. Reuse a viable solution meeting the project criteria; adapt or extend it where that closes a specific gap. Author new code when no adequate solution exists or when a focused experiment demonstrates a significant advantage. Owning the engine architecture does not require owning every algorithm's implementation.

Make subsystems modular with explicit ownership, data and scheduling contracts. Prefer independently testable Rust modules/crates and replaceable backend boundaries without mandating dynamic plugins, a permanent public ABI or runtime indirection in hot loops.

Prefer suitable Rust physics libraries as the first evaluation path. Jolt is an exception requiring major workload-relevant advantages over viable Rust alternatives after including bindings, data conversion, build complexity and maintenance. Name recognition, familiarity or a marginal microbenchmark gain is insufficient. No physics library is selected by this ADR.

Develop Rust bindings, adapters, tooling or hardware-specific backends when necessary or substantially beneficial. Reuse existing API bindings where adequate. Such interfaces use capabilities exposed by Android, drivers and vendor SDKs; they cannot create missing hardware capabilities or assume privileged access. A missing Rust wrapper is a research/integration task, not an automatic reason to abandon Rust.

## Alternatives

A C++ default no longer reflects owner intent. A Rust-only purity rule could waste effort on viable foreign libraries and tools. Automatic adoption of any available crate would ignore quality/performance criteria. Rewriting everything would contradict the explicit reuse requirement. A universal abstraction over every possible backend is not required for modularity.

## Consequences

The project includes targeted systems/tooling research as needed, with responsibility for testing and maintaining any new interfaces. Safe Rust is preferred; unsafe GPU/FFI boundaries require explicit invariants, resource retirement and error-handling contracts. A safety wrapper must not claim that Rust proves GPU synchronization or foreign-library correctness.

Use a small working baseline to identify actual gaps. A bounded prototype tests a credible advantage before production adoption; proof of a gain is not required before running the experiment. Avoid speculative infrastructure work or an unbounded survey that prevents implementation.

## Validation

Follow [component selection](../COMPONENT_SELECTION.md) for substantial dependency/custom-code decisions and exceptions. Establish workload-specific thresholds before performance comparisons; include full-frame/step costs, quality, memory, thermals and compatibility. Record adopt/adapt/build/defer outcomes and revisit conditions. Hardware-dependent gains remain unproven until tested on physical hardware; independent development may proceed provisionally.

## References

[Requirements](../REQUIREMENTS.md), [superseded foundation](0003-native-foundation.md), [superseded reuse proposal](0004-build-and-reuse-boundary.md), [Rust foundation proposal](0015-rust-native-foundation.md), [physics proposal](0010-physics-and-editing.md).
