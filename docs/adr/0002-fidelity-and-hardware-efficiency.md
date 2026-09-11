# ADR-0002: Visual ambition and sustained hardware efficiency

- Date: 2026-09-07
- Status: Accepted
- Basis: Explicit owner priorities; efficiency interpretation documented below.
- Requirements: R03, R04, R07, R08, R09

## Context

The owner wants high-end Android visual fidelity, complex physics and effective use of CPU, GPU, multicore processing, RAM and NPU capabilities. Desired graphics include Lumen-like dynamic lighting and Nanite-like detail variation.

## Decision

Preserve high-end fidelity, complex physics and effective hardware use as architectural goals. Optimize useful delivered quality and simulation under sustained device limits. Interpret maximum hardware use as exploiting capabilities when beneficial, with evidence; it does not require consuming all memory or saturating every processor continuously.

Plan high-end device profiles first and measure actual capabilities. Exact frame-rate, resolution, memory and device-floor targets remain open engineering choices. Lighting/detail goals do not promise exact Unreal implementations or feature parity.

## Alternatives

Maximum utilization alone is an inadequate success metric. Lowest-common-denominator compatibility would compromise the requested high-end focus if imposed prematurely. Unconditional use of every advanced feature may consume more budget than it saves.

## Consequences

Thermal, memory, pacing and total workload costs matter alongside image quality. Hardware-specific paths need explicit requirements or fallbacks. Accelerator experiments must include data movement and synchronization. Complex simulation must fit alongside rendering rather than receiving only leftover resources.

## Validation

Use actual-device sustained results and matched image quality under [the benchmark protocol](../BENCHMARKS.md). Record adopted and deferred capabilities honestly. Proposed numeric budgets are not owner-approved promises.

## References

[Android thermal feedback](https://developer.android.com/games/optimize/adpf/thermal), [advanced hardware proposal](0013-advanced-hardware-and-research.md).
