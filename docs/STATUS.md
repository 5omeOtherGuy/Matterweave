# Current status

Updated: 2026-09-07

## Repository setup

The repository was empty when initially inspected. The setup added a self-contained brief, requirements, 13 ADRs, research, architecture, milestones, benchmarks and handoff. The subsequent Rust policy update brings the repository to 15 ADRs and 20 requirements, with an explicit component-selection procedure.

Documentation tooling and templates are the only executable/project support artifacts in this setup. **No engine code, native application, APK, integration benchmark or device run exists yet.**

## Decisions

- Accepted: ADR-0001 (product/platform), ADR-0002 (fidelity and efficient hardware use), ADR-0012 (validation/continuity), ADR-0014 (Rust wherever feasible without detriment, modularity, qualifying reuse and justified custom technology/interfaces).
- Proposed: ADR-0005 through ADR-0011, ADR-0013 and ADR-0015 (Rust native foundation). No specific physics/rendering package is selected.
- Superseded: ADR-0003/0004. Their historical C++/reuse recommendations must not guide new foundation selection. Earlier browser-demo constraints also remain non-binding.
- Physics now starts by assessing suitable Rust libraries such as Rapier. Jolt is eligible only for major advantages after complete integration costs. No Jolt comparison is required absent a credible major gap in qualifying Rust options.

## Validation at setup

Run `python3 tools/check_docs.py` to check local Markdown links, ADR structure/index/status consistency and requirement-to-ADR references. This checks repository documentation integrity, not the correctness or performance of a future engine. The setup is reviewed for requirement/proposal distinctions and explicit definitions of done.

Original setup check on 2026-09-07: **PASS — 29 Markdown files, 97 local links, 13 ADRs and 16 requirements.** Rust-policy update check on the same date: **PASS — 32 Markdown files, 128 local links, 15 ADRs and 20 requirements.** The documentation workflow runs the same check on pushes and pull requests; consult the live GitHub check for its execution result.

There are no Android build or performance results to report. Native build/test CI is to be introduced in M0. External research sources are recorded with an assessment date; they are not vendored dependencies or performance results for Matterweave.

## Next actions

1. Start M0 under accepted ADR-0014: inspect build/device access, assess qualifying existing components, pin the Rust/Android toolchain and capability profile, and resolve ADR-0015 for the first build. Do not reopen the superseded C++ default.
2. Create the Android shell, touch/lifecycle handling and diagnostics; produce an APK and exact clean-checkout build instructions.
3. Advance to M1's queryable/editable voxel slice and tests, maintaining a reference implementation for correctness.
4. Add real-device evidence when available, then use M2 comparisons to choose the primary representation/rendering path.

## Known limitations and owner questions

No reference device has been inventoried or reserved for this project. No physical-device access is established by this repository setup. Numeric budgets are provisional. The project license is undecided; no license has been selected for the owner. See [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) for timing and working defaults.

## Update format for implementation sessions

Replace stale status with a concise account of the current milestone, implemented behavior, relevant commit/artifact identifiers, exact commands and results, device/run metadata, remaining gaps and next tasks. Preserve detailed experiment outcomes in ADRs or linked benchmark reports; do not leave passing claims unsupported by evidence.
