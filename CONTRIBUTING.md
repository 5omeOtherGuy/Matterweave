# Contributing to Matterweave

Read [AGENTS.md](AGENTS.md) and the [ADR index](docs/adr/README.md) first. Work state and next milestones live in issues and pull requests.

## Changes

- Explain the problem, resulting behavior and relevant requirement/ADR.
- Keep engine services separate from game-specific content.
- Prefer Rust where feasible without material detriment. Reuse qualifying components; justify substantial custom work, foreign dependencies and interfaces through component selection, recorded in an ADR. Jolt requires major advantages over viable Rust alternatives.
- Preserve unrelated work and avoid force-pushing shared branches.
- Resolve routine technical decisions with evidence; record material decisions in an ADR using [the template](docs/adr/template.md).
- Record sources, exact versions, licenses and local changes for adopted
dependencies and assets. A research link is not an adopted dependency.
- The owner has not selected a project license. Do not add one or represent this repository as licensed open source without that decision.

## Verification

Run `python3 tools/check_docs.py` for documentation changes. Use the native/Android
checks documented in [the development guide](docs/DEVELOPMENT.md) as implementation
adds them. Report tests run and relevant tests not run with reasons. Performance
claims carry the measurement conditions [AGENTS.md](AGENTS.md) requires; visual-fidelity
claims use the metrics in [tools/fidelity](tools/fidelity/README.md).

## Pull requests

Put what you changed, the validation actually run and its limits in the PR description, using the [pull request template](.github/pull_request_template.md). Do not describe a build-only result as device-tested or a target as a measurement.
