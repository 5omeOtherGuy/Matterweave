# Working in this repository

## Profile
- Purpose: native Android voxel engine and reusable game framework in Rust, with sample games that validate it.
- Stage: pre-release (repository standard 2026-10-17)
- Gate: `scripts/gate` (CI job `gate`)
- Landing: PR, squash-merged on a green gate; agents land: yes
- Release: none (the owner publishes GitHub prereleases); agents release: no
- Outside issues and PRs: agents triage; outside PRs merge: owner approves
- Deliberate deviations from the standard: every change lands through a PR, even in
  pre-release, and `.githooks/pre-push` refuses direct pushes to `main`; a PR also
  needs the `host` (Vulkan smoke) and `android` (APK) jobs green, which stay outside
  the hermetic gate; a few workspace tests still wait on worker threads with bounded
  sleeps (#106); claims only
  for what ran, measurements with their conditions, authoritative world data, layer
  separation, smallest slice, Rust and reuse (Rules below); owner-only decisions
  (Delivery below); no per-task, status or handoff documents (Writing things down
  below); component selection, ADRs and dependency/asset provenance, and the
  measurement rule for performance claims ([CONTRIBUTING.md](CONTRIBUTING.md)).

Matterweave is a native Android voxel engine and reusable game framework. The
engine is the product; the sample games exist to validate it. Android-native
delivery is what counts — a desktop or host build is a development aid, never
evidence that something works.

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the layout,
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) to build and test, and
[docs/adr/README.md](docs/adr/README.md) for decisions already made and why.
Then read the code you are about to change.

Engine crates live in `crates/`, the explorer and Mossbound hosts in `apps/`, the
Gradle projects in `android/`, the patched winit in `vendor/winit` and helper scripts
in `tools/`. Run `.agents/setup` once per checkout: it installs the pinned toolchain
and the gate's secret scanner and enables `.githooks`. A focused test is
`cargo test --locked -p <crate> --test <file> <name>`.

## Rules

**Don't claim what you didn't run.** A green host test says nothing about the
phone. A successful APK build says nothing about the app working. Target
budgets, synthetic traces and unexecuted commands are not results. Say "not
run" — it is always an acceptable answer and never a failure.

**Measurements need their conditions.** Device, OS, driver, scene, seed, build
configuration and commit, or it is not a measurement. Never infer phone
behaviour from desktop or emulator numbers.

**World data is authoritative.** Render, lighting and collision structures are
derived and versioned. A change to visual detail must never silently move a
wall or change a game rule.

**Keep the layers apart.** Android lifecycle, engine core, and game rules stay
separate. Vulkan and physics types do not leak through gameplay interfaces.

**Smallest slice that works.** Build the vertical slice the current goal needs.
No editor, plugin system or general framework ahead of a working native path.

**Rust, and reuse before writing.** Check for an existing crate before building
your own; pin what you adopt. See ADR-0014 and ADR-0015.

## Delivery

Branch, commit, push, open a PR, fix what CI and review find, merge. That is
pre-authorized — don't stop at a local commit and don't ask again each time.
Never force-push shared history.

Branch as `<type>/<slug>`, one concern per PR, with a Conventional Commits title.
`scripts/gate` must pass locally (the pre-push hook runs it). Merge once the latest
`gate`, `host` and `android` runs on the head commit all concluded `success`:
`gh pr merge <number> --squash --delete-branch --match-head-commit <sha>`.
Changes to CI or release configuration get an independent review first.

Ask the owner only for a decision that is genuinely theirs: hardware, licensing,
repository access, a store release, or anything irreversible. Anything needing
the physical phone is theirs too. State what is blocked and keep working on the
rest.

## Writing things down

Put what you did in the PR description, next to the diff.

Do not add a file per task, per worker or per review. Do not write status,
handoff or progress documents. If a change makes existing text wrong, edit that
text — deleting a stale paragraph beats adding a correct one beside it.

This repository once carried 225 markdown files and 26,000 lines of prose for
seven days of work, most of it written once and never read. The code, the tests
and the PR history are the durable record. If they are not enough to resume
from, fix them rather than writing prose about them.
