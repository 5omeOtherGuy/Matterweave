# detail-production-fixtures — engineering log `w_18ada142`

Slice D1.2 (M2-A): representative production vegetation and occlusion/movement
coverage in the native automatic-detail diagnostic. Workspace
`/mnt/bench/matterweave-dev/worktrees/detail-production-fixtures`, branch
`engine/detail-production-fixtures`, base `39666a8`. Owned paths only:
`apps/explorer/src/detail_check.rs` and this log. No device access, no push, no
PR, no lead-owned integration or acceptance. No performance claim.

Read first: [D1.1 detail runtime](d1-1-detail-runtime.md),
[native detail check](../detail-native-check.md),
[engine completion plan](../../ENGINE_COMPLETION_PLAN.md) D1.2.

## Declared criteria and results

| Criterion | Verification | Owner | Result |
| --- | --- | --- | --- |
| Real production flora and eligible coarse positive control in the native diagnostic | Source-derived fixture tests using `matterweave_detail::showcase_prototype`; per-phase selected LOD/source revision probes | worker | PASS |
| Occlusion-oriented views, moved instance and edit source/version invalidate; thin geometry retained | Discriminating host behavioral tests plus in-run assertions; projected-AABB occlusion and declared probe deltas | worker | PASS |
| Existing cases and zero-extent/lifecycle safety retained | 16 focused tests, 154 app tests, 578 workspace tests, scoped strict Clippy, fmt | worker | PASS |
| Actual rendered host run | `--detail-check` under llvmpipe with validation; report and run log hashed below | worker | PASS (host software Vulkan only) |
| Android visual/functional acceptance and independent review | Lead after handoff; no quality claim from CPU checks | lead | NOT RUN |

## What changed

`apps/explorer/src/detail_check.rs` only.

- **Representative production-flora corpus.** Six species are constructed by
  the exported production constructor `showcase_prototype` (the same entry point
  the showcase scatter uses): `funnel_mushroom` (hollow-pit collidable fungus),
  `fan_frond`, `reed_cluster`, `twisted_shrub` (collidable woody root, decorative
  crown), `horsetail` and `marsh_lily`. No local look-alike geometry is defined.
  The fixture is now 11 prototypes / 15 instances with negative coordinates and
  all four yaws.
- **Occlusion-oriented view.** Phase `occlusion-foreground-fungus-over-solid`
  looks along `x = -20` so the thin fungus at 92 m depth overlaps a dense boulder
  at 96 m depth. The in-run check requires the fungus to be `Source` while its
  `Half` error estimate still passes the pixel budget (i.e. the geometry guard,
  not distance, retained it), and requires the rear boulder to select and realize
  `Half`/`Quarter` in the same frame. A unit test additionally projects both
  instance AABBs through the phase view-projection and asserts screen overlap and
  depth order.
- **Bounded lazy realization instead of preload-only.** The diagnostic now warms
  the authoritative `Source` meshes once (production `warm_source` semantics) and
  realizes coarse levels only while a camera selects them, under a local
  `MAX_COARSE_BUILDS_PER_PREPARE = 2` mirroring the wetland policy. Three distinct
  eligible `(prototype, lod)` pairs exist at both the 480 host and 1080 Android
  viewport heights (fine boulder at two depths, second fine dense control, 0.25 m
  tile-scale dense control), so the first far prepare always defers at least one
  pair; phase 0 `cold-far-bounded-convergence` repeats the identical stationary
  prepare until the deferred set is empty and then proves a zero-build steady
  state. `resident-zero-build-budget` (cap 0, far camera) now runs *before* the
  fixture rebuild and reports zero builds with zero fallbacks.
- **Instance movement with correct invalidation.** The move phase rebuilds the
  authoritative scene with the same prototypes at the shrub's new placement:
  `source_version` advances while every voxel revision is unchanged, so the
  resident pool must not invalidate (`geometry_changed == false`) and only packed
  placements change; the in-run check also requires the prepared instance to
  carry the new translation. The moved-instance edit phase then removes the
  shrub's root cell, advancing its revision and forcing a revision-matched
  derived refresh (`geometry_changed == true`).
- **Source-authoritative probes.** Seven named world-metre points (dense core,
  edited boulder cell, occlusion solid core, fungus stipe, hollow fungus pit,
  shrub home root, shrub moved root) are captured as `(collidable, sample)` and
  re-checked after every phase. Camera/LOD phases must leave them byte-identical;
  the move and the two edits must change exactly their declared probe names. The
  emptied pit (`None`) is direct evidence that a visual coarse fill of thin flora
  can never change the authoritative answer.
- **Quality guards preserved.** No threshold is relaxed for selection. Every
  production flora instance is asserted `Source` in every phase; sheet and
  opening remain `Source`; the dense positive controls must realize non-empty
  coarse meshes whose revision equals the live source.
- **Compact phase/LOD HUD.** Each rendered frame draws three lines (phase index
  and name; projection + `Source/Half/Quarter` histogram + deferred + builds +
  convergence iterations; flora retention and move/edit state), so Android
  screenshots are self-identifying.
- **Eager fixture capability check.** `check_fixture_levels` runs
  `DetailRuntime::preload` once on a disposable `fork_source` and requires all
  11 × 3 derived levels to be buildable, so an unbuildable fixture level can
  never masquerade as a retained quality guard. The run's resident pool stays
  lazy; this keeps the previously used `preload` API live without editing the
  non-owned `detail_runtime.rs`.

Phase count changed from 9 to 14. Android holds each phase for 120 presented
frames (1680 total). The lead's device procedure and expected phase count need
updating in the lead-owned docs.

## Host behavioral tests

`cargo test -p matterweave-explorer --lib detail_check` — 16 passed:

- `fixture_realizes_every_prototype_at_every_level`
- `stale_runtime_is_rejected_after_source_edit`
- `preload_builds_each_lod_once_with_source_revisions`
- `native_fixture_preserves_opening_while_dense_control_coarsens`
- `approach_selects_source_and_retreat_coarsens_the_same_instance`
- `thin_sheet_stays_at_source_across_camera_phases`
- `source_probes_are_camera_and_lod_invariant`
- `edit_advances_source_version_and_rebuild_matches_new_revision`
- `budget_cap_falls_back_to_source_on_a_fresh_scene_and_maps_to_source_indices`
- `instances_map_selection_to_resident_indices_with_transform`
- `production_flora_corpus_is_held_at_source_where_pixels_would_allow_coarse`
- `default_quality_guards_not_distance_hold_every_flora_species`
- `occlusion_phase_layers_guarded_flora_in_front_of_a_realized_coarse_solid`
- `moving_a_flora_placement_updates_instances_without_rebuilding_geometry`
- `editing_the_moved_instance_invalidates_and_refreshes_its_geometry`
- `cold_lazy_convergence_is_bounded_and_reaches_a_zero_build_steady_state`

Guard attribution probe (part of the second test, diagnostic only — the relaxed
configs are never rendered; every species stays `Source` under the default
config at a camera where its coarse level passes the pixel budget):

```text
flora_fungus  (funnel_mushroom) default=Source guard=local_loss relaxed=Quarter
flora_frond   (fan_frond)       default=Source guard=dilation   relaxed=Half
flora_reed    (reed_cluster)    default=Source guard=dilation   relaxed=Half
flora_shrub   (twisted_shrub)   default=Source guard=dilation   relaxed=Half
flora_spire   (horsetail)       default=Source guard=dilation   relaxed=Quarter
flora_lily    (marsh_lily)      default=Source guard=dilation   relaxed=Half
```

This answers the recorded production gap (`8302 Source / 0 Half / 0 Quarter`
under `max_local_loss_fraction = 0`): retention of thin flora is a justified
quality-guard outcome, while eligible dense geometry in the same fixture, camera
and frame does realize `Half`/`Quarter`.

## Verification commands and results

```sh
CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/coarse-terrain/target-detail-production \
CARGO_BUILD_JOBS=2 cargo test -p matterweave-explorer --locked --lib detail_check
# exit 0: 16 passed, 0 failed

CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/coarse-terrain/target-detail-production \
CARGO_BUILD_JOBS=2 cargo test -p matterweave-explorer --locked
# exit 0: 154 passed, 0 failed, 1 ignored (pre-existing ignored wetland full-map test)

CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/coarse-terrain/target-detail-production \
CARGO_BUILD_JOBS=2 cargo test --workspace --locked
# exit 0: 578 passed, 0 failed, 3 ignored

CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/coarse-terrain/target-detail-production \
CARGO_BUILD_JOBS=2 cargo fmt -p matterweave-explorer -- --check
# exit 0

CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/coarse-terrain/target-detail-production \
CARGO_BUILD_JOBS=2 cargo clippy -p matterweave-explorer --all-targets --locked -- -D warnings
# exit 0 (one pre-existing vendored winit warning; no explorer warnings)
```

## Actual rendered host run

```sh
MATTERWEAVE_VALIDATION=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  xvfb-run -a cargo run --locked -p matterweave-explorer -- --detail-check \
  --save /mnt/bench/matterweave-dev/coarse-terrain/detail-production-fixtures/unused.json
# exit 0; report ends "PASS detail ..."
```

Device: llvmpipe (LLVM 20.1.2, 256 bits), Vulkan 1.4.318, validation layer on.
Report: `/mnt/bench/matterweave-dev/coarse-terrain/detail-production-fixtures/detail-check-report.txt`
sha256 `d279d1b6608a30bf36c602849ca2195543b5b16cf7d15bd4fb34adf9d6c5e098`.
Run log: `/mnt/bench/matterweave-dev/coarse-terrain/detail-production-fixtures/detail-check-run.log`
sha256 `4e8bec3f16ae478d1b614a4fa54cf03e9ec6a795819aba2b1f64e4b230e89e38`.

Observed phase evidence (host `detail-check-report.txt`):

- Phase 0 `cold-far-bounded-convergence`: `converge_iters=2`,
  `max_prepare_builds=2`, `total_converge_builds=3`, `coarse_realized=7` — the
  bounded lazy path defers and then converges, realizing both `Half` and
  `Quarter` with matching source revisions.
- Phase 7 `occlusion-foreground-fungus-over-solid`: `flora_source=6/6`,
  `flora_pixel_eligible=6`, `fallback=0`, rear solid `Quarter` realized in the
  same frame.
- Phase 9 `resident-zero-build-budget`: `builds_this_call=0`, `fallback=0` —
  nothing outstanding after convergence.
- Phase 10 `move-flora-instance`: `geometry_changed=false`, pooled instances
  updated to the moved translation (asserted in-run); `builds_this_call=11` is
  the fixture scene rebuild's source-mesh cache (a new `DetailScene` starts with
  an empty cache); it is not a runtime geometry invalidation, which
  `geometry_changed=false` and the unit test's pool-size assertion pin.
- Phase 11 `move-edit-invalidation`: `geometry_changed=true`, shrub revision
  `r500 -> r501`, probe delta exactly `shrub_target_root`.
- Phase 12 `edit-invalidate-rebuild`: `geometry_changed=true`, boulder revision
  `r512 -> r513`, probe delta exactly `solid_edit_cell`.
- Phase 13 `lifecycle-recreate`: zero extent returns `Retry`, the renderer is
  recreated and re-warms an 11-entry `Source`-only pool.
- Terminal line: `PASS detail: 14 phases ... Source/Half/Quarter ...`.

`MATTERWEAVE_VALIDATION=1` reported no validation errors. This is software
Vulkan functional evidence only; it is not an Android or performance result.

## Decisions and rationale

- **No guard relaxation.** The positive control is separate eligible dense
  geometry (`dense_control`, `dense_tile_control`, boulder instances). Thin
  production flora staying at `Source` is asserted and attributed, never worked
  around by raising `max_local_loss_fraction` or `max_dilation_fraction`.
- **Lazy over preload.** The prior diagnostic preloaded every level, so the
  zero-build-budget phase could not show bounded realization. The run now warms
  `Source` only and the first phase proves deferral, bounded builds per prepare,
  progress, convergence and a zero-build steady state, matching the wetland's
  `pending`/`deferred` policy.
- **Movement as authoritative re-placement.** `DetailScene` has no transform
  mutation API, so a move is expressed the way the detail system sees it: the
  authoritative scene is rebuilt with the same prototypes at a new placement.
  Voxel revisions are deterministic and unchanged, which is exactly the
  placement-only case the runtime must handle without geometry invalidation.
  The subsequent edit covers the invalidating case.
- **`preload` as fixture capability check.** Removing it from the frame path
  left the public runtime API unused in non-test builds (Clippy `-D dead-code`).
  Editing the non-owned `detail_runtime.rs` was out of scope, so the diagnostic
  uses `preload` on a disposable fork once to prove the fixture builds all
  levels. A fixture that silently degraded a level would otherwise make the
  guard assertions untrustworthy.
- **Zero-cap phase before the move.** After the fixture rebuild the new scene's
  mesh cache is cold, so a cap-0 phase would legitimately fall back. The phase
  now runs while every selected level is resident, making `fallback=0` and
  `builds=0` a truthful stationarity guarantee; cold-cache cap fallback remains
  covered by `budget_cap_falls_back_to_source_on_a_fresh_scene_...`.

## Issues and friction

- The first host run failed `finalize` with "fewer than two LODs were ever
  chosen": the rewrite dropped the `lods_seen` insertion. Restored in
  `check_frame`; the rerun passes.
- The move test initially asserted `mesh_builds_this_call == 0`. That counter is
  `DetailScene`'s derived-mesh cache (per `prepare_batches`), and the rebuilt
  fixture starts with an empty cache, so it rebuilds 11 `Source` meshes. The
  assertion now pins the runtime-level invariant (`geometry_changed == false`,
  unchanged resident pool) and the report exposes `geometry_changed` per phase.
- `DetailRuntime::preload` became dead code for the non-test build after the
  lazy switch (strict Clippy). Resolved inside owned paths with the fixture
  capability check described above.
- Pre-existing vendored `winit` warning `function_casts_as_integer` remains; it
  is not part of this slice and does not fail the scoped gate.

## Not verified / remaining gates

- **Android OnePlus 13 (lead-owned, NOT RUN):** install and run
  `engine-check.txt = detail`, confirm all 14 phases, view the occlusion, move
  and edit phase screenshots, and independent review. Phase count and capture
  schedule changed; lead updates the device procedure.
- **Visual quality (lead-owned, NOT RUN):** the occlusion claim in this slice is
  per-instance LOD plus projected-AABB overlap and depth order, not a pixel
  readback or human quality judgement. No coarse-transition/zoom visual
  acceptance is claimed.
- **Performance (NOT RUN):** no timing or memory claim; host software Vulkan is
  not device performance evidence.
- **Bounds:** the bounded-convergence phase relies on at least three distinct
  coarse pairs at the declared 480 host / 1080 Android viewport heights; the
  unit test pins the 1080 case. An unusual runtime window height could reduce
  the pair count and fail the phase explicitly (never silently pass).
