# Instructions for agents and contributors

## Mission and reading order

Build Matterweave: a native Android voxel engine and reusable game framework with high visual fidelity, complex interactive physics and measured efficiency. Start with [docs/HANDOFF.md](docs/HANDOFF.md), [docs/STATUS.md](docs/STATUS.md), [docs/REQUIREMENTS.md](docs/REQUIREMENTS.md), then [docs/adr/README.md](docs/adr/README.md) and [docs/ROADMAP.md](docs/ROADMAP.md). Read the relevant implementation files before changing them.

This repository is the durable project context: the code, the tests, the ADRs and the pull request history. Record a decision when a future reader would otherwise repeat a mistake, and record it in the smallest durable place — a code comment, a test name, an ADR, or a PR description. Do not depend on prior conversations, personal memory, external agent mailboxes or another project's files. Working notes, transcripts and per-attempt narration are not project context and do not belong in this repository.

## Authority and autonomy

- Use subagents for bounded independent tasks. The coordinating agent owns integration, consistency and verification. Avoid overlapping edits; communicate concrete deliverables and shared interface changes.
- Advance through working implementation and relevant verification. Do not stop after planning, scaffolding or creating a backlog when the authorized implementation task can proceed.
- Resolve ordinary engineering choices with evidence and record them. Proposed ADRs permit experiments; they are not automatic requests for owner approval. Follow the decision process in the ADR index.
- Preserve explicit owner requirements. Ask only when an actual missing owner decision, inaccessible capability or irreversible action prevents the next necessary step. Continue independent work and state precisely what remains blocked.
- The owner authorizes the normal delivery workflow: commit completed work on a branch, push it, open a pull request, resolve actionable review/CI findings, and merge after the relevant checks pass. Do not stop at a local commit or an unmerged PR, and do not ask for repeated permission for these steps. Preserve branch protections and shared history; report any access or required-review blocker.
- Do not infer authority to purchase hardware/services, change repository visibility or access controls, select the owner's license, publish a store release, or overwrite others' work. Follow the active session's Git and integration authorization; do not force-push shared history.

## Scope and decision integrity

- Android-native delivery is mandatory. A web preview or desktop executable is supporting evidence only.
- The engine is the reusable product. Genre examples are validation scenarios, not instructions to implement a Pokémon game, copy assets or import rules from an unrelated project.
- The earlier browser/site demo constraints were explicitly replaced. A serene alien forest is an optional showcase, not a fixed art direction or a world-size limit.
- Voxels are required as meaningful world/object data. Ray rendering is an important candidate, but ray-only rendering was not re-established as a mandatory constraint after the scope reset. Preserve the rendering comparison in ADR-0006.
- Lumen-like lighting and Nanite-like detail are requirements at the outcome level. Proposed algorithms and libraries are not approved results, shipped features or performance guarantees.
- Separate accepted requirements, working engineering decisions, hypotheses and measurements. Never mark an ADR accepted on the owner's behalf if it changes product intent.

## Engineering and verification

- Follow accepted [ADR-0014](docs/adr/0014-rust-modularity-and-evidence-led-reuse.md): Rust wherever feasible without material detriment, explicit modularity and reuse of viable alternatives meeting strict criteria. Use [component selection](docs/COMPONENT_SELECTION.md) for substantial decisions. The former C++ default is superseded by accepted [ADR-0015](docs/adr/0015-rust-native-foundation.md).
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

Before handing off, run relevant checks and inspect the diff. Put what you did in the pull request description: what changed, what was tested, what is still open. GitHub keeps that next to the diff, where the next reader is already looking.

Do not create a per-task, per-worker or per-review file. Do not append a dated entry to [docs/STATUS.md](docs/STATUS.md) for routine work. Update a document only when the change makes something already written **wrong**, and then edit that document in place rather than adding a new one beside it. Deleting a stale paragraph is worth more than adding a correct one next to it.

A fresh session resumes from the code, the tests and recent pull requests. If that is not enough, fix the code or the tests — not by writing more prose about them.

Document the device/OS/driver, scene, seed, build configuration, commit and test conditions for performance claims. Keep large binary evidence in appropriate repository release/workflow artifacts with durable references and checksums; keep small manifests and summaries in Git.

For documentation changes, run `python3 tools/check_docs.py`. Add native build/test/CI commands to [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) as they become real.
