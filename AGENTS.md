# Instructions for agents and contributors

## Mission and reading order

Build Matterweave: a native Android voxel engine and reusable game framework with high visual fidelity, complex interactive physics and measured efficiency. Start with [docs/HANDOFF.md](docs/HANDOFF.md), [docs/STATUS.md](docs/STATUS.md), [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md), then [docs/adr/README.md](docs/adr/README.md) and [docs/ROADMAP.md](docs/ROADMAP.md). Read the relevant implementation files before changing them.

This repository is the durable project context. Preserve decisions, evidence, useful failures and reproducible commands here. Do not depend on prior conversations, personal memory, external agent mailboxes or another project's files.

## Authority and autonomy

- The owner requested this repository setup and a subsequent session orchestrating implementation. During implementation, use available subagents for bounded independent tasks when helpful; the coordinating agent owns integration, consistency and verification. Avoid overlapping edits, and communicate concrete deliverables and shared interface changes.
- Advance through working implementation and relevant verification. Do not stop after planning, scaffolding or creating a backlog when the authorized implementation task can proceed.
- Resolve ordinary engineering choices with evidence and record them. Proposed ADRs permit experiments; they are not automatic requests for owner approval. Follow the decision process in the ADR index.
- Preserve explicit owner requirements. Ask only when an actual missing owner decision, inaccessible capability or irreversible action prevents the next necessary step. Continue independent work and state precisely what remains blocked.
- Do not infer authority to purchase hardware/services, change repository visibility or access controls, select the owner's license, publish a store release, or overwrite others' work. Follow the active session's Git and integration authorization; do not force-push shared history.

## Scope and decision integrity

- Android-native delivery is mandatory. A web preview or desktop executable is supporting evidence only.
- The engine is the reusable product. Genre examples are validation scenarios, not instructions to implement a Pokémon game, copy assets or import rules from an unrelated project.
- The earlier browser/site demo constraints were explicitly replaced. A serene alien forest is an optional showcase, not a fixed art direction or a world-size limit.
- Voxels are required as meaningful world/object data. Ray rendering is an important candidate, but ray-only rendering was not re-established as a mandatory constraint after the scope reset. Preserve the rendering comparison in ADR-0006.
- Lumen-like lighting and Nanite-like detail are requirements at the outcome level. Proposed algorithms and libraries are not approved results, shipped features or performance guarantees.
- Separate accepted requirements, working engineering decisions, hypotheses and measurements. Never mark an ADR accepted on the owner's behalf if it changes product intent.

## Engineering and verification

- Follow accepted [ADR-0014](docs/adr/0014-rust-modularity-and-evidence-led-reuse.md): Rust wherever feasible without material detriment, explicit modularity and reuse of viable alternatives meeting strict criteria. Use [component selection](docs/COMPONENT_SELECTION.md) for substantial decisions. The former C++ default is superseded; the current foundation proposal is ADR-0015.
- Inspect suitable existing solutions before substantial custom implementation. New technology, tools and Rust hardware interfaces are authorized where necessary or significantly advantageous. Prototype credible improvements, then validate before claiming or adopting a performance advantage. Do not rewrite adequate dependencies solely for language uniformity.
- Evaluate suitable Rust physics first. Jolt is eligible only for major workload-relevant advantages after binding, data conversion, build and maintenance costs. No physics package is selected by policy alone.
- Keep unsafe/FFI/GPU contracts narrow and explicit. Logical modularity need not impose runtime plugins, a permanent ABI, large copies or dynamic dispatch in inner loops. Missing Rust bindings are engineering tasks; new wrappers do not create hardware capabilities or privileged device access.
- Prefer the smallest complete vertical slice that resolves the current milestone. Introduce abstractions for actual requirements; avoid building an editor, plugin ecosystem or general rendering framework before the native slice works.
- Pin adopted dependencies and build tools; record source, revision, license and integration rationale. Avoid floating dependency branches in reproducible builds.
- Keep Android UI/lifecycle, engine core and game-specific rules separate. Do not leak Vulkan or physics-library types through every public gameplay interface.
- Treat world data as authoritative; render, lighting and collision representations are derived and versioned. Changes to visual LOD must not silently change game rules or remove physical walls.
- Account for CPU/GPU synchronization, staging buffers, shared-memory pressure, edit invalidation and thermal behavior. More threads, RAM allocation or accelerator usage is not proof of better performance.
- Verify meaningful risks: data round trips, chunk boundaries, negative coordinates, stale asynchronous jobs, edits, resource lifetimes, lifecycle recovery, collision updates and visual temporal stability. Avoid tests that only mirror implementation details.
- Keep baseline workloads and comparison settings reproducible. Never infer mobile performance from desktop/emulator numbers. Record unavailable hardware tests as not run.
- Do not present synthetic traces, target budgets or unexecuted commands as measured results. A successful APK build is not proof of successful device operation.

## Completion and handoff

Before handing off, run relevant checks, inspect the diff, and update [docs/STATUS.md](docs/STATUS.md) with exact accomplishments, commands/results, unresolved risks and next actions. Update affected ADRs and the roadmap. A fresh session must be able to resume from the repository alone.

Document the device/OS/driver, scene, seed, build configuration, commit and test conditions for performance claims. Keep large binary evidence in appropriate repository release/workflow artifacts with durable references and checksums; keep small manifests and summaries in Git.

For documentation changes, run `python3 tools/check_docs.py`. Add native build/test/CI commands to [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) as they become real.
