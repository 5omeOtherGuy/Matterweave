# Working in this repository

Matterweave is a native Android voxel engine and reusable game framework. The
engine is the product; the sample games exist to validate it. Android-native
delivery is what counts — a desktop or host build is a development aid, never
evidence that something works.

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the layout,
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) to build and test, and
[docs/adr/README.md](docs/adr/README.md) for decisions already made and why.
Then read the code you are about to change.

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
