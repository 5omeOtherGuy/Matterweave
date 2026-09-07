# ADR-0012: Reproducible evidence and repository continuity

- Date: 2026-09-07
- Status: Accepted
- Basis: Owner's repository-only handoff requirement and setup engineering convention.
- Requirements: R03, R13, R14

## Context

Implementation will continue in a new session with the repository as its only project context. Performance architecture needs trustworthy measurements; documentation or a successful build cannot establish physical-device behavior.

## Decision

Keep requirements, ADRs, live status, commands, dependency/toolchain provenance and next actions in Git. Maintain clear distinctions between owner goals, proposals, accepted engineering decisions, implemented behavior and measurements. Pin adopted dependencies and record their licenses.

Use staged native milestones and reproducible fixture-based comparisons. Separate host tests, package builds, emulator checks, physical-device functional checks and sustained performance evidence. Label targets and unavailable tests honestly. Update relevant records at implementation handoffs.

## Alternatives

Chat-only plans, unversioned local notes and undocumented agent coordination cannot satisfy repository-only continuity. FPS averages or visually impressive stills alone cannot validate dynamic quality and thermal efficiency. Excessive process should not displace useful code.

## Consequences

Documentation must evolve with source rather than becoming an initial specification that drifts. Use concise reports and relevant tests; retain important failures. Hardware access can block a validation claim while leaving substantial development work available.

## Validation

The setup validator checks local documentation links, ADR structure/status indexing and requirement references. M0 adds actual build/test CI; later milestones attach device/run evidence. A fresh checkout/session must follow repository instructions without retrieving prior chat.

## References

[Agent instructions](../../AGENTS.md), [development guide](../DEVELOPMENT.md), [benchmark protocol](../BENCHMARKS.md), [handoff](../HANDOFF.md).
