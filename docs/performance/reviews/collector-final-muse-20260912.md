Engineering log — collector-final review (read-only, no edits/builds)

Frozen worktree: `/mnt/bench/matterweave-dev/worktrees/collector-final-review-20260912` (e436634). Diff read: `/mnt/bench/matterweave-dev/completion-20260912/collector-final.diff` (40a0844..e436634, `tools/performance` only: `collect_wetland_pair.py`, `analyze_wetland_pairs.py`, both test files). No other reviews/logs read.

App recovery source inspected: `apps/explorer/src/wetland_state.rs:43-86` (`validate`, `load` 2 MiB limit), `:92-164` (`load_recovering_with`: `Ok(None)` on missing base → never scans slots; highest-valid-first selection; invalid retained), `apps/explorer/src/wetland.rs:315-330` (runtime passes `GENERATOR`/`SEED` + real `apply_journal` restore), `crates/matterweave-detail/src/showcase.rs:37` (`SHOWCASE_GENERATOR_VERSION=3`).

Focus checks (all pass, no findings):
- Manifest/fixture/session consistency: `resolve_generator` (`collect_wetland_pair.py:996`) rejects bool/non-int/unsupported/mixed generators pre-device; `check_fixture(raw,generator)` (`:244`) pins fixture to declared generator with bool-safe compare; `finish_and_pull(...,generator)` (`:706`) re-validates pulled `session-after.json` against same generator. Cross-generator fixture accepted-then-ignored failure mode is closed.
- Full-world scene counts: `KNOWN_COMPOSITIONS` (`:93-97`) gen3 `dfb9f40519a3c151`→34716467/8302 verified in `docs/evidence/full-wetland-generator3.json:10,42-43`; gen2 `f458591e7b345546`→34864520/8324 consistent with `docs/performance/logs/completion-execution.md:161`. `resolve_expected_scene` (`:1028`) cross-checks composition↔generator and rejects unknown/mixed/partial `expected_scene` overrides, with bool-safe int validation.
- Early mismatch rejection: `main` (`:1080-1084`) order is `resolve_generator` → `resolve_expected_scene` → `check_fixture` → manifest write → `Device(...)` construction; `Device.__init__` is arg-storage only, first device I/O is in `run_trial`. `StartupRejectionTest` patches `Device` with a forbidden stub, proving no device contact on fixture/manifest disagreement.
- Valid user base-save protection: `preflight_files` (`:506-529`) rejects missing base (`Ok(None)` → fixture ignored per `load_recovering_with`), rejects base valid for declared generator (user session would load, fixture ignored), rejects outranking slot 128, foreign profile request/gallery marker/foreign fixture slot; `ownership.claim` only after all rejects; `final_cleanup` (`:887`) removes only `ownership.owned`. `base_save_is_invalid_for` (`:298`) is fail-closed (checks generator+version only; seed/restore failures also block rather than overwrite). New tests cover valid-current-base untouched and stale-gen2-base reaching slot 127.
- ON-only capture stability + pair cardinality: `pair_members` (`analyze_wetland_pairs.py:278`) `len!=2→None` closes 3-member duplicate collapse; `pair_mismatches` (`:305-311`) requires every present capture internally single-valued (catches unstable ON in ON/OFF pair) and cross-compares only when both exist. Tests cover both.

Findings: none. No source-proven defect in the corrected paths; foreign-file protection is not relaxed (only worker-owned slot 127 + owned profile request are ever written/removed).

NOT RUN: no device/phone, build, test, or acceptance execution performed in this leaf; lead independently reproduces and owns acceptance.

