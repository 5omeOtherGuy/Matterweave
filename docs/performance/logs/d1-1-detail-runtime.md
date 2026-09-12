# d1-1-detail-runtime — engineering log (2026-09-12)

Slice D1.1: production automatic-detail runtime module and its tests.
Workspace `/mnt/bench/matterweave-dev/worktrees/d1-1-detail-runtime`, base
`7dace54` on `phase-a/d1-1-detail-runtime`. No device access, no production
wiring (both lead-owned). Nothing under `crates/matterweave-detail/**` needed
changing.

## Definition of done

| Criterion | Verification | Result |
| --- | --- | --- |
| `detail_runtime` exposes a runtime type owning resident derived geometry and returning `Vec<StaticInstance>` from `prepare_batches` output | `detail_runtime::tests::prepare_maps_selection_to_resident_geometry` (+ 6 more runtime tests) | PASS |
| Editing a prototype's source refreshes affected geometry; instances afterwards reference the new revision, never a stale one | `edit_refreshes_selected_geometry_and_never_hands_out_stale_revisions`, `emptied_prototype_is_reported_instead_of_drawn_from_stale_geometry` | PASS |
| Near camera selects a finer level than a far camera through `prepare_batches` (histogram differs) | `near_camera_selects_a_finer_level_than_far_camera`: near=`Source`, far=`Quarter`; `lod_histogram` differs | PASS |
| `detail_check.rs` delegates with no duplicated catalog/instance-mapping logic; existing behaviour unchanged | Diff removes `StaticCatalog`, `yaw_quarters`, `instances_for_frame`, `lod_histogram`; 9 `detail_check` tests pass unchanged in outcome | PASS |
| Nothing mutates source voxels or physics state | Diff touches only `detail_runtime.rs`, `detail_check.rs`, log; `prepare_never_mutates_the_authoritative_scene` asserts source version and `SceneCounts` unchanged | PASS |
| `cargo test --workspace --locked` | exit 0; 44 test binaries, 501 passed, 0 failed, 3 ignored | PASS |
| `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` | fmt clean; clippy exit 0 | PASS |
| Production wiring of `wetland.rs::graphics()` | lead-owned | NOT RUN |
| Android approach/retreat and edit captures on the OnePlus 13 | lead-owned; no device access | NOT RUN |

Supplementary (not a DoD row): host detail gate
`--detail-check` under `VK_ICD_FILENAMES=lvp_icd.json` (llvmpipe LLVM 20.1.2
Vulkan 1.4.318, validation on) now runs through the runtime and prints
`PASS detail` for all 9 phases, exit 0, no validation errors. Report/log:
`/mnt/bench/matterweave-dev/performance/run-d11/detail-check/`
(`detail-check-report.txt` sha256
`cbddca73c5776781b59e1e63bf4c1e6734c195a9f0bc19f70de21e427b17f2f6`,
`detail-check-run.log` sha256
`2fabc364d10b98078e93dc1bea7f6197b200a958cb4de97d85ca477ef4d1df9d`).
The phase-6 edit moves the boulder from r512 to r513 and the selected meshes
follow; the zero-build-budget and lifecycle-recreate phases still build 0.
This is host software Vulkan correctness evidence only, not device performance.

## Actions Taken

- Read `detail_check.rs` in full, plus `scene.rs` (`prepare_batches`,
  `ensure_mesh`, `prototype_mesh`, `cached_prototype_mesh`, `edit_prototype`,
  `source_version`), `select.rs`, and the renderer's `StaticInstance` /
  `replace_static_scene` / `update_static_instances` contracts.
- Wrote `detail_runtime.rs` tests first; `cargo test -p matterweave-explorer
  detail_runtime` failed to compile on the missing `DetailRuntime`,
  `RESIDENT_LODS`, `yaw_quarters`, `lod_histogram` (compile-time RED), then
  implemented the module (GREEN).
- API: `DetailRuntime::preload(scene)` realizes every level of every nonempty
  prototype and verifies the derived revision matches the source;
  `DetailRuntime::new()` is empty/lazy for callers that want only selected
  levels built. `prepare(scene, camera, config)` calls
  `DetailScene::prepare_batches`, refreshes the selected stale copies from the
  freshly built scene-cache meshes, verifies `frame.source_version` against the
  live scene, and maps the selection to `StaticInstance`s. The returned
  `FrameUpdate { frame, instances, geometry_changed }` tells the renderer
  whether it needs a static-scene re-install or an instance-only update.
- Delegated `detail_check.rs`: removed `StaticCatalog`, `yaw_quarters`,
  `instances_for_frame` and `lod_histogram`; `LODS` now aliases
  `RESIDENT_LODS`; `install_static_scene`/`apply_phase` call
  `DetailRuntime::preload`/`prepare`; tests use `runtime.instances_for_frame`.
- Found and fixed a stale-handout edge case while reviewing the refresh path:
  a prototype emptied by an edit kept its old resident entry, so the next frame
  would map its instance to the pre-edit mesh. `refresh_selected` now drops a
  resident prototype with zero occupied cells, so `instances_for_frame` reports
  it instead of returning stale geometry. Regression test added (concrete
  case: remove all 512 cells of a placed dense boulder; `prepare` errors and
  the index is gone).
- Ran the project's documented host detail gate, which now exercises the
  delegated runtime end-to-end in the real renderer.

## Issues & Friction

- `cargo clippy --workspace --all-targets --locked -- -D warnings` rejected an
  unused `DetailRuntime::source_version()` accessor (`-D dead-code`). The
  accessor was speculative API surface with no consumer, so it was removed
  rather than kept with an allow. No behaviour lost: `prepare` owns version
  handling internally.
- The leader's working tree already contained the uncommitted
  `apps/explorer/src/lib.rs` line `mod detail_runtime;` (excluded from edits).
  It had to be included in this commit, otherwise the committed `detail_check`
  could not reference `crate::detail_runtime`. `lib.rs` was not edited.
- No material tooling friction. The vendored `winit` emits one pre-existing
  `function_casts_as_integer` warning that is not part of this slice and does
  not fail the gate (`clippy` exit 0).

## Decisions & Rationale

- Refresh is selection-driven, not eager: only the `(prototype, lod)` pairs the
  engine selected for the frame are refreshed, and only after
  `prepare_batches` has built them within `LodConfig::max_coarse_builds`. An
  eager "refresh all levels of the edited prototype" pass would build coarse
  meshes behind the cap and break the zero-build-budget contract. A coarser
  level of an edited prototype is refreshed by the next frame that selects it;
  it can never be handed out stale because the resident revision is checked
  against the live source before mapping.
- Empty prototypes are dropped from the resident pool rather than stored as
  vertexless meshes. Storing one would let `prepare` succeed and push the
  failure into `replace_static_scene`, which is transactional and could leave a
  previous, now-stale static scene installed. Erroring in `prepare` keeps the
  "never hand out stale geometry" invariant at the module boundary.
- `instances_for_frame` keeps the source-version equality guard from the old
  adapter. `prepare` updates that version and refreshes before mapping, so the
  guard only rejects frames prepared against a different scene state (misuse or
  a caller bypassing `prepare`), which is exactly the one-shot-gate behaviour
  the existing test pinned.
- `preload` on an empty scene returns an empty runtime instead of the adapter's
  "fixture produced no nonempty prototypes" error. Production can legitimately
  have no prototypes yet; the detail-check fixture always has three, so its
  behaviour is unchanged.
- `lod_histogram` moved to `detail_runtime` (used by both the check report and
  the runtime tests), keeping one copy.

## Solutions Applied

- `apps/explorer/src/detail_runtime.rs` (new contents): `RESIDENT_LODS`,
  `yaw_quarters`, `lod_histogram`, `FrameUpdate`, `DetailRuntime` with
  `new`/`preload`/`prepare`/`instances_for_frame`/`meshes`/`instance_index`/
  `revision`, plus 7 unit tests.
- `apps/explorer/src/detail_check.rs`: delegate to the runtime; net −118 lines;
  no catalog, yaw encoding, mapping or histogram logic remains local.
- `docs/performance/logs/d1-1-detail-runtime.md` (this file).

Verification commands and results:

```sh
cargo test -p matterweave-explorer --locked
# exit 0: 96 tests, 95 passed, 1 ignored (pre-existing), including 7 runtime tests
cargo test --workspace --locked
# exit 0: 501 passed, 0 failed, 3 ignored across 44 test binaries
cargo fmt --all -- --check
# exit 0
cargo clippy --workspace --all-targets --locked -- -D warnings
# exit 0
MATTERWEAVE_VALIDATION=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  xvfb-run -a cargo run --locked -p matterweave-explorer -- --detail-check \
  --save /mnt/bench/matterweave-dev/performance/run-d11/detail-check/unused.json
# exit 0; report ends "PASS detail ... 9 phases"
python3 tools/check_docs.py
# exit 0: PASS: 184 Markdown files, 581 local links, 16 ADRs and 20 requirements.
```

## Insights

- The old adapter's failure mode (refuse a version-mismatched frame) is the
  right guard for the gate but the wrong production response; keeping the guard
  in `instances_for_frame` while moving refresh into `prepare` preserves both
  behaviours with one code path.
- Per-prototype revision comparison, not `SceneVersion` alone, is what makes a
  refresh affordable: `place()` advances the version without changing any
  derived geometry, and that must not trigger a re-upload.
- The host detail gate is a fast, honest way to exercise the whole LOD→GPU path
  (llvmpipe, validation on) without touching the device; it caught nothing here
  but is the natural regression net for the lead's `wetland.rs` wiring.

## Not verified / assumptions

- Android behaviour, frame cost and visual quality: not run; lead-owned.
- Wetland-scale preload cost (880 prototypes x 3 levels) was not measured; the
  lazy `new()` path exists for callers that prefer selection-limited builds, and
  the lead chooses the wiring. Host gate timing is not a device-performance
  claim.
- A `DetailRuntime::new()` caller that never calls `prepare` cannot map frames
  (`source_version` differs); that is the documented misuse error, not a
  supported flow.

## Correction pass (same day, follow-up commit)

Four defects were reported against `f56eaf1` and fixed in
`apps/explorer/src/detail_runtime.rs` plus three `detail_check` test call sites.
TDD order: all four regression tests were written first and run against
`f56eaf1` with `--no-fail-fast`; all four failed for the intended reason, then
the fixes made them pass.

### 1. Emptying a prototype no longer breaks `prepare`

- `refresh_selected` no longer drops the resident entry. A selected level whose
  revision moved on is always refreshed through `refresh_entry`, including the
  empty derived mesh of an emptied prototype, which reuses the pool slot.
- `instances_for_frame` treats a prototype with `occupied_cells() == 0` as
  undrawable and omits its instance; the frame continues. No zero-vertex mesh
  reaches the renderer.
- RED evidence: `emptied_prototype_contributes_no_instance_and_refill_restores_geometry`
  panicked at the emptied `prepare(...).unwrap()` because mapping returned the
  old `"has no resident geometry"` error.
- The test now proves: the emptied boulder is still in `frame.selected` but
  contributes no instance; the independent keeper instance is still returned
  with a current revision; refilling one cell restores a non-empty mesh and the
  boulder's instance.

### 2. No public path maps a superseded revision

- `instances_for_frame` now takes `scene: &DetailScene` and validates each
  selected `(prototype, lod)`'s resident revision against the prototype's live
  source revision before mapping. The whole-scene `source_version` guard alone
  cannot catch the partial-refresh case, and the old public signature made the
  contract unenforceable.
- RED evidence: `partially_refreshed_runtime_rejects_a_foreign_frame` failed
  `assert!(runtime.instances_for_frame(&far).is_err())` — after an edit and a
  near `prepare` (Source refreshed), a directly prepared far frame selected the
  still-stale Quarter copy and the old guard mapped it.
- With the fix the same frame is rejected; adding `&scene` was the only change
  the test needed.

### 3. `preload` degrades like `ensure_mesh`

- `refresh_entry` now returns `Ok(false)` for `DetailError::InvalidScale` and
  `DetailError::BudgetExceeded` instead of propagating them; `preload` and
  `refresh_selected` leave that level non-resident and selection falls back to
  `Source`, exactly as `DetailScene::ensure_mesh` does.
- Verified fact for the record: wetland terrain is `TERRAIN_CELL_M =
  SCALE_TILE_M = 0.25 m` (not 0.05 m), and `Lod::Quarter` coarsens by 4 giving
  exactly 1.0 m = `MAX_SCALE_M`, accepted only because the bound is inclusive.
  Any coarser prototype, or a scene near the 64 MiB derived-mesh cache budget,
  would have failed `preload` outright before this fix.
- Two tests: `preload_degrades_when_a_coarse_level_is_out_of_scale_range`
  (0.3 m cells, Quarter = 1.2 m; preload succeeds with Source+Half resident and
  Quarter absent, and a very far camera still selects and draws Half) and
  `preload_degrades_when_a_level_exceeds_the_mesh_budget` (33^3 cells cross the
  32 MiB derived-mesh output upper bound before allocation; every level stays
  non-resident and preload succeeds). The BudgetExceeded arm was already RED on
  `f56eaf1` through the scale test; the mesh-budget test adds direct coverage of
  the second error variant.

### 4. Empty/refill cycles reuse pool slots

- The emptied prototype keeps its `ResidentPrototype` entry and stores the empty
  mesh at the existing index, so refilling replaces that slot in place.
- `empty_and_refill_cycles_reuse_pool_slots` runs three clear/refill cycles and
  asserts `runtime.meshes().len()` stays at the 3 slots preloaded for the
  boulder. On `f56eaf1` the emptied entry was removed and refill pushed a new
  slot per cycle; the cycle test never reached that assertion because the
  emptied `prepare` returned `Err` first, so the growth was inferred from the
  removal/append code rather than measured on the old revision.

### Correction definition of done

| Criterion | Result |
| --- | --- |
| Emptying leaves `prepare` succeeding; no instance for the emptied prototype; others correct; refill restores | PASS |
| No public path maps a superseded revision; partial-refresh test fails on `f56eaf1` | PASS (RED captured) |
| `preload` degrades on InvalidScale/BudgetExceeded; out-of-scale Quarter still preloads | PASS (two tests) |
| Empty/refill does not grow the pool | PASS |
| `cargo test --workspace --locked` | PASS (505 passed, 0 failed, 3 ignored) |
| `cargo fmt --all -- --check`, clippy `-D warnings` | PASS |
| Correction accepted against the source | NOT RUN (lead) |
| Android OnePlus 13 approach/retreat and edit | NOT RUN (lead) |

Supplementary: the host `--detail-check` gate rerun after the correction still
prints `PASS detail` (exit 0, validation on, llvmpipe LLVM 20.1.2). Evidence:
`/mnt/bench/matterweave-dev/performance/run-d11/detail-check-correction/`
(report sha256 `cbddca73c5776781b59e1e63bf4c1e6734c195a9f0bc19f70de21e427b17f2f6`,
run log sha256 `c1cf9185650997ddc1762bb22dae602cc3aa77224394c7127ef60095e0f52927`).
