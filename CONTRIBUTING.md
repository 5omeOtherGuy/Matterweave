# Contributing to Matterweave

Read [AGENTS.md](AGENTS.md), [the implementation handoff](docs/HANDOFF.md) and [the ADR index](docs/adr/README.md) first. The project is currently in pre-implementation; contribute towards the next concrete milestone in [the roadmap](docs/ROADMAP.md).

## Changes

- Explain the problem, resulting behavior and relevant requirement/ADR.
- Keep engine services separate from game-specific content.
- Preserve unrelated work and avoid force-pushing shared branches.
- Resolve routine technical decisions with evidence; record material decisions in an ADR using [the template](docs/adr/template.md).
- Record sources, exact versions, licenses and local changes for adopted dependencies/assets. A research link is not an adopted dependency.
- The owner has not selected a project license. Do not add one or represent this repository as licensed open source without that decision.

## Verification

Run `python3 tools/check_docs.py` for documentation changes. Use the real native/Android checks documented in [the development guide](docs/DEVELOPMENT.md) as implementation adds them. Report tests run and relevant tests not run with reasons. Performance claims follow [the benchmark protocol](docs/BENCHMARKS.md).

## Handoffs and pull requests

Keep [STATUS.md](docs/STATUS.md) current. State implemented behavior, actual validation, limitations and next work. Link durable artifacts where useful. Do not describe a build-only result as device-tested or a target as a measurement. Use the [pull request template](.github/pull_request_template.md) when opening a PR.
