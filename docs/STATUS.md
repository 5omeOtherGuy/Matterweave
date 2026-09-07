# Current status

Updated: 2026-09-07

## Repository setup

The repository was empty when inspected. The setup adds a self-contained product brief, requirements and traceability, 13 architecture decision records, research comparisons, architecture boundaries, development guidance, staged delivery criteria, benchmark protocol and implementation handoff.

Documentation tooling and templates are the only executable/project support artifacts in this setup. **No engine code, native application, APK, integration benchmark or device run exists yet.**

## Decisions

- Accepted: ADR-0001 (product/platform), ADR-0002 (fidelity and efficient hardware use), ADR-0012 (reproducible validation and repository continuity).
- Proposed: ADR-0003 through ADR-0011 and ADR-0013. These preserve concrete starting recommendations without falsely attributing technology selections to the owner.
- None superseded within this ADR series. Earlier browser-demo constraints are historical, explicitly non-binding context.

## Validation at setup

Run `python3 tools/check_docs.py` to check local Markdown links, ADR structure/index/status consistency and requirement-to-ADR references. This checks repository documentation integrity, not the correctness or performance of a future engine. The setup is reviewed for requirement/proposal distinctions and explicit definitions of done.

Local setup check on 2026-09-07: **PASS — 29 Markdown files, 97 local links, 13 ADRs and 16 requirements.** The documentation workflow runs the same check on pushes and pull requests; consult the live GitHub check for its execution result.

There are no Android build or performance results to report. Native build/test CI is to be introduced in M0. External research sources are recorded with an assessment date; they are not vendored dependencies or performance results for Matterweave.

## Next actions

1. Start M0: inspect build/device access, select and pin the native toolchain and minimal Android capability profile, resolve ADR-0003/0004 for the first build.
2. Create the Android shell, touch/lifecycle handling and diagnostics; produce an APK and exact clean-checkout build instructions.
3. Advance to M1's queryable/editable voxel slice and tests, maintaining a reference implementation for correctness.
4. Add real-device evidence when available, then use M2 comparisons to choose the primary representation/rendering path.

## Known limitations and owner questions

No reference device has been inventoried or reserved for this project. No physical-device access is established by this repository setup. Numeric budgets are provisional. The project license is undecided; no license has been selected for the owner. See [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md) for timing and working defaults.

## Update format for implementation sessions

Replace stale status with a concise account of the current milestone, implemented behavior, relevant commit/artifact identifiers, exact commands and results, device/run metadata, remaining gaps and next tasks. Preserve detailed experiment outcomes in ADRs or linked benchmark reports; do not leave passing claims unsupported by evidence.
