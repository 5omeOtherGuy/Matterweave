# Contributing to Matterweave

Read [AGENTS.md](AGENTS.md) and the [ADR index](docs/adr/README.md) first. Work state and next milestones live in issues and pull requests.

Small fixes are welcome directly as a pull request; larger changes start with an
issue. Issues and pull requests are triaged on a best-effort basis while the project
is pre-release. Report vulnerabilities as [SECURITY.md](SECURITY.md) describes.

## Changes

- Explain the problem, resulting behavior and relevant requirement/ADR.
- Keep engine services separate from game-specific content.
- Prefer Rust where feasible without material detriment. Reuse qualifying components; justify substantial custom work, foreign dependencies and interfaces through component selection, recorded in an ADR. Jolt requires major advantages over viable Rust alternatives.
- Preserve unrelated work and avoid force-pushing shared branches.
- Resolve routine technical decisions with evidence; record material decisions in an ADR using [the template](docs/adr/template.md).
- Record sources, exact versions, licenses and local changes for adopted
dependencies and assets. A research link is not an adopted dependency.
- Contributions are licensed under the repository's license, MIT OR Apache-2.0 (see the [README](README.md#license)); there is no CLA.

## Verification

Run `.agents/setup` once, then `scripts/gate` before you push; CI runs the same command
in its `gate` job, and every pull request must pass it. Use the native/Android
checks documented in [the development guide](docs/DEVELOPMENT.md) as implementation
adds them. Report tests run and relevant tests not run with reasons. Performance
claims carry the measurement conditions [AGENTS.md](AGENTS.md) requires; visual-fidelity
claims use the metrics in [tools/fidelity](tools/fidelity/README.md).

## Pull requests

Put what you changed, the validation actually run and its limits in the PR description, using the [pull request template](.github/pull_request_template.md). Do not describe a build-only result as device-tested or a target as a measurement.
