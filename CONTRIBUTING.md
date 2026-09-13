# Contributing to Matterweave

Read [AGENTS.md](AGENTS.md), the implementation handoff and the
[ADR index](docs/adr/README.md) first. For current state and the next milestone, see
STATUS and the roadmap.

## Changes

- Explain the problem, resulting behavior and relevant requirement/ADR.
- Keep engine services separate from game-specific content.
- Prefer Rust where feasible without material detriment. Reuse qualifying components; justify substantial custom work, foreign dependencies and interfaces through component selection. Jolt requires major advantages over viable Rust alternatives.
- Preserve unrelated work and avoid force-pushing shared branches.
- Resolve routine technical decisions with evidence; record material decisions in an ADR using [the template](docs/adr/template.md).
- Record sources, exact versions, licenses and local changes for adopted
dependencies and assets. A research link is not an adopted dependency.
- The owner has not selected a project license. Do not add one or represent this repository as licensed open source without that decision.

## Verification

Run `python3 tools/check_docs.py` for documentation changes. Use the native/Android
checks documented in [the development guide](docs/DEVELOPMENT.md) as implementation
adds them. Report tests run and relevant tests not run with reasons. Performance
claims follow the benchmark protocol.

## Handoffs and pull requests

Keep STATUS.md current: implemented behavior, actual validation,
limitations and next work. Link durable artifacts where useful. Do not describe a
build-only result as device-tested or a target as a measurement. Use the
[pull request template](.github/pull_request_template.md) when opening a PR.
