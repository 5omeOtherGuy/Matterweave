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
