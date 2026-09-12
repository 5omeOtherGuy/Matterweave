### Review Log: Diff 40a0844..e436634 (`tools/performance`)

**Target Commit**: `e436634` (worktree `/mnt/bench/matterweave-dev/worktrees/collector-final-review-20260912`)  
**Scope**: `tools/performance/collect_wetland_pair.py`, `tools/performance/analyze_wetland_pairs.py`, and corresponding unit tests (`test_collect_wetland_pair.py`, `test_analyze_wetland_pairs.py`).  
**Authority**: Read-only Pi leaf. No edits, builds, phone execution, or acceptance authority.

---

### Verification and Focus Areas

1. **Generator 2/3 Manifest, Fixture, and Session Consistency**
   - `collect_wetland_pair.py:996-1015` (`resolve_generator`): Manifest declarations are strictly validated (`int`, non-boolean, within `SUPPORTED_GENERATORS = (2, 3)`), and must match across all input APKs (`len(set(declared.values())) == 1`).
   - `collect_wetland_pair.py:244,259-261` (`check_fixture`): Fixture generator must match the manifest-declared generator (`generator != declared or isinstance(..., bool)` raises `ValueError`).
   - `collect_wetland_pair.py:756-758` (`finish_and_pull`): The pulled `session-after.json` is re-validated via `check_fixture(session, generator)`, guaranteeing session-after matches the declared world generator.
   - Alignment with app source: `crates/matterweave-detail/src/showcase.rs:37` defines `SHOWCASE_GENERATOR_VERSION: u32 = 3`, used by `apps/explorer/src/wetland.rs:36` (`const GENERATOR: u32 = matterweave_detail::SHOWCASE_GENERATOR_VERSION;`). Collector's `CURRENT_GENERATOR = 3` matches runtime reality while retaining generator 2 support for historical benchmark reproducibility.

2. **Full-World Scene Counts**
   - `collect_wetland_pair.py:97-100` (`KNOWN_COMPOSITIONS`):
     - `dfb9f40519a3c151`: Generator 3 (`cells: 34,716,467`, `instances: 8,302`), matching host evidence record `docs/evidence/full-wetland-generator3.json:42-43`.
     - `f458591e7b345546`: Generator 2 (`cells: 34,864,520`, `instances: 8,324`), matching `docs/evidence/full-wetland-generator2.json:42-43` and `docs/performance/measurement-qualification.md:96`.
   - `collect_wetland_pair.py:1018-1046` (`resolve_expected_scene`): Enforces that composition hash generator matches the declared manifest generator (`known["generator"] == generator`), preventing cross-generator composition mixing.
   - `collect_wetland_pair.py:611` (`enter_wetland`): `check_scene_counts(loaded, expected_scene)` asserts that logcat `WETLAND LOADED: {cells} cells / {instances} placed objects;` matches `expected_scene` exactly; smaller/sub-scenes raise `TrialError`.

3. **Early Mismatch Rejection**
   - `collect_wetland_pair.py:1062-1085`: All preflight gates (`expected_apk_metadata`, `resolve_generator`, `resolve_expected_scene`, `check_fixture`) execute before `Device(...)` is instantiated at line 1098.
   - Fixture/manifest/composition disagreements raise exceptions immediately; no ADB connections or device files are touched. Verified by `StartupRejectionTest` in `test_collect_wetland_pair.py:558-603`.

4. **Valid User Base-Save & Foreign File Protection**
   - App recovery semantics (`apps/explorer/src/wetland_state.rs:114-160`): `load_recovering_with` inspects base `wetland-session.json` first. If missing, it starts a fresh session without scanning recovery slots. If valid, it restores the base session without scanning recovery slots. It scans recovery slots `(1..=128).rev()` only if the base save fails validation or restoration.
   - Collector preflight (`collect_wetland_pair.py:521-529`):
     - Missing base save raises `TrialError` (prevents fixture from being ignored).
     - Valid base save for the active generator raises `TrialError` (`base_save_is_invalid_for` returns `False`), preventing user save clobbering or unreachable fixtures.
     - Invalid base save (e.g. generator 2 base under generator 3 run) allows the run to proceed to slot 127 (`wetland-session.json.recovery-127.json`).
   - Foreign file isolation: Pre-existing slot 128 (`OUTRANKING_SLOTS`) or pre-existing slot 127 aborts the run without touching device state (`collect_wetland_pair.py:516-533`). `final_cleanup` (`collect_wetland_pair.py:898-917`) removes only files tracked in `ownership.owned`; user `world.json`, base saves, and existing recovery slots are never deleted or modified.

5. **ON-Only Capture Stability**
   - `analyze_wetland_pairs.py:303-311` (`pair_mismatches`): Collects all non-None captures across `(a, b)`. For ON/OFF pairs where OFF has no capture CSV, `captures` has 1 item.
   - `any(len(values) != 1 for values in sets)` validates internal capture stability for the ON trial across `gpu_prev_shadows`, `gpu_prev_shadow_map_size`, and `voxel_bodies_total`. Fluctuations during capture trigger mismatch rejection even when unpaired with another capture CSV. Cross-trial stability comparison (`sets[0] != sets[1]`) runs when both trials hold captures.

6. **Pair Cardinality**
   - `analyze_wetland_pairs.py:278-279` (`pair_members`): Explicit guard `if len(members) != 2: return None` guarantees only exact 2-trial groups form pairs. Trios or duplicate runs cannot collide through dict comprehensions into false pairs.

---

### Findings

**No findings.**  
All 6 examined areas adhere to engine recovery contracts, file protection invariants, and cross-generator consistency without regressions.

---

### Device Execution Statement

**NOT RUN**: No device connection, ADB commands, APK builds, or hardware executions were performed by this review agent (read-only inspection under assigned authority). Lead independently reproduces and owns acceptance.
