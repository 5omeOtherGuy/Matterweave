# ADR-0013: Capability-based acceleration and frontier research

- Date: 2026-09-07
- Status: Proposed
- Basis: Proposed engineering response to the owner's modern-hardware ambition and frontier-technology question.
- Requirements: R04, R16

## Context

The owner explicitly wants useful access to modern Android CPU, GPU, RAM and NPU capabilities. Support, programmability, driver behavior and workloads vary. Adding an accelerator can introduce conversion, transfer, synchronization or thermal costs that exceed its benefit.

## Decision

Inventory actual device/API features early. Evaluate multicore scheduling, SIMD, GPU-driven work, shader precision, subgroups, asynchronous compute, hardware RT and NPU paths against an existing workload. Adopt per-capability paths only with a defined contract and total-cost evidence; report unsupported features clearly.

For NPUs, first identify a useful inference workload and available runtime/vendor API, then measure input/output movement, execution, synchronization and quality. Do not assume an NPU is a general-purpose Vulkan graphics unit. Keep CPU/GPU baseline paths where practical.

Reserve neural radiance caches, ReSTIR-family sampling, frame generation and advanced simulation for bounded later experiments. These remain opportunities; none is required before the first useful native renderer.

## Alternatives

Ignoring advanced capabilities would miss the owner's ambition. Requiring all of them in the launch baseline would tie progress to unproven dependencies. Hardware-specific forks throughout the codebase would make maintenance difficult.

## Consequences

Feature profiles need to describe actual capabilities and tested behavior. Hardware RT may require a different acceleration representation for voxels, with build/refit costs included. NPU integration must show an application-level benefit, not merely successful inference. Negative results are useful evidence.

## Validation

M0 records capabilities; later experiments specify workload, baseline, device coverage, total costs, quality, fallback and adopt/defer/reject outcome. No acceleration path is claimed implemented in this setup.

## References

[Hardware efficiency goal](0002-fidelity-and-hardware-efficiency.md), [research](../RESEARCH.md), [benchmarks](../BENCHMARKS.md).
