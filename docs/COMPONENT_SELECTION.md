# Component selection and custom technology

This implements the owner's accepted [Rust/modularity/reuse policy](adr/0014-rust-modularity-and-evidence-led-reuse.md). Apply it to substantial components and exceptions. Routine small implementation details do not require a separate architecture record or exhaustive survey.

## Selection order

1. Define the capability and constraints in terms of actual Matterweave workloads.
2. Inspect the strongest relevant existing Rust solutions and their source/integration requirements.
3. Reuse a candidate meeting the criteria. Consider a focused adapter or upstream extension when a specific gap remains.
4. Consider a foreign dependency under the language-exception policy, including integration costs. For Jolt, require major advantages over viable Rust alternatives.
5. Prototype custom technology when no adequate solution exists or there is a credible significant advantage. Adopt it only with appropriate correctness and comparative evidence.

This is not a rule to prefer a weak Rust implementation over a substantially better solution, nor to rewrite adequate tools for language uniformity. Compare adapting an existing solution, a justified foreign component and custom development when more than one route is credible. A Rust crate being available does not establish that it meets our criteria.

## Strict criteria

| Dimension | Evidence to collect |
| --- | --- |
| Required behavior | Correctness, needed algorithms/features, physical/visual quality and world-edit semantics. |
| Android capability access | Actual target/API/driver support, extension access, packaging and lifecycle compatibility. |
| Speed and latency | Total frame/physics/job time, tails/stalls and update latency at matched quality. |
| Efficiency | Resident/peak memory, allocation and copy traffic, synchronization, sustained thermal/power behavior where measurable. |
| Modularity | Clear ownership/data contracts, ability to replace a backend and integration without spreading vendor types throughout the engine. |
| Reliability | Relevant tests, unsafe/FFI invariants, error recovery and maintenance history. |
| Integration and maintenance | Glue, build/toolchain burden, patches, dependency footprint, licensing compatibility and ongoing maintenance responsibility. |

There is no universal invented percentage defining significant or major. Before a comparison, set a workload-specific threshold and explain why crossing it matters: meeting the frame budget, supporting a required interaction, reducing memory enough to enable a scene, or materially improving stability/quality. A small isolated speedup that disappears after integration is insufficient.

## Rust and foreign interfaces

New code defaults to Rust where feasible without material detriment. Record exceptions for necessary platform/vendor interfaces or demonstrated advantages. A narrow binding can be preferable to reimplementing an adequate mature library. A custom interface must identify the exposed Android/Vulkan/vendor API, the missing or costly existing layer and the expected improvement. It cannot assume access to an undocumented capability, replace a device driver by declaration or promise zero-copy paths without supported memory/synchronization semantics.

Document ownership, data representation, resource lifetime, execution threads, errors/panics and asynchronous completion. Measure all marshaling, conversions, dispatches and transfers. Prefer reusable Rust-facing interfaces; keep unsafe implementation details localized. Test replacement of a component when a real second implementation is evaluated, without imposing runtime plugin machinery on every subsystem.

## Physics exception

Start by assessing suitable Rust physics, with Rapier a current candidate. Jolt is eligible only for major advantages in relevant performance, capabilities, robustness or physical fidelity after accounting for the Rust binding and long-term integration. Mere familiarity or a C++ ecosystem preference is not enough. If the Rust baseline meets the criteria and no credible major gap is identified, do not delay development for a full Jolt integration experiment.

## Experiment and decision record

Record the component, requirement, candidate/version/license, existing-solution assessment, gap/hypothesis, acceptance thresholds, benchmark fixture, total costs, correctness/quality result, device evidence, maintenance owner and adopt/adapt/build/defer conclusion. Use [the benchmark protocol](BENCHMARKS.md) for runtime claims. Keep reports concise and link them from affected ADRs.

When hardware is unavailable, document a provisional integration decision and unresolved performance claim. Build the minimum experiment needed to resolve a credible gap; do not require proof of a future gain before writing a prototype, and do not call the prototype a proven optimization. Prefer upstreaming reusable fixes where feasible, but external publication or outreach is not an automatic milestone requirement.
