# Engineering log w_355788a2

Worker: `deepseek-flash-go` · outcome: a small executable scene actually renders fine plus derived coarse terrain using existing Renderer/winit bootstrap from instancing_smoke.rs, then edits and rederives it. This must draw coarse-sized mesh faces, not expand coarse cells into many fine cubes and call it LOD. Reuse World::mesh on a bound · run: `/mnt/bench/matterweave-dev/coarse-terrain/visual-implementation`

## Outcome

Complete for this worker's scope. `crates/matterweave-render/examples/terrain_tiles_smoke.rs`
is a standalone example that

1. owns one authoritative non-streaming `World` (deterministic hillside,
   seed `0xc0a57e115eed0001`),
2. meshes it as an all-fine reference through the unchanged `World::mesh`,
3. derives a level-2 `World::coarse_tile` for the eastern half and adapts it to
   world-space geometry with an example-only tile adapter, combining it with a
   bounded fine copy of the western half into one aggregated mesh uploaded with
   `Renderer::upload`,
4. removes a coarse-sized (4x4x4 fine cells = exactly one level-2 coarse cell)
   mineral feature from the authoritative world and re-derives/re-meshes both
   representations,
5. presents four frames from one fixed camera and terminates deterministically
   (optional `--hold` keeps the window open).

LOD never mutates the authoritative source world; the coarse
adapter builds a fresh 16^3 tile-local world per tile and only reads the source
world. No core, renderer, app, manifest or CI file was modified. No rendering,
LOD, collision, Android or performance adoption claim is made.

## Files (owned)

- New: `crates/matterweave-render/examples/terrain_tiles_smoke.rs`
- New: `docs/performance/logs/coarse-terrain-visual.md` (this log)

The worker timed out before committing. Lead integrates these two files after
review; the provisional core
dependency copy (`crates/matterweave-core/src/coarse.rs`, `lib.rs`,
`streaming.rs`) is read-only here and was not staged.

## Geometry contract implemented

Adapter (`coarse_tile_mesh`): copies tile materials into a fresh tile-local
`World` (local index `x + 16 * (y + 16 * z)`), calls the unchanged
`World::mesh`, then bakes `tile.origin()` and the `1 << level` cell scale into
every vertex. Authoritative state is never passed in and cannot be mutated.
`exposed_faces` reimplements only the mesher's air/outside culling rule to
check the result.

CPU tests (all in the example, `#[cfg(test)]`):

- `coarse_adapter_keeps_coarse_face_scale` — one quad per exposed coarse face,
  two triangles per quad, every vertex on the coarse lattice at the tile
  origin, and no triangle edge shorter than one coarse cell (2 at level 1,
  4 at level 2). A fine-expanded adapter (`(1 << level)^3` cubes) fails the
  edge and face-count assertions. Also checks the feature is one coarse cell at
  both levels and that its level-2 top is a single quad of two triangles.
- `mixed_scene_is_fine_left_and_coarse_right` — triangles left of the split
  have unit edges, triangles starting at the split have coarse-cell edges; the
  scene bounds hold; the bounded copy provably generates on-plane duplicate
  wall faces and the mixed fine side removes every one the coarse side owns.
- `derivation_is_read_only_and_repeatable` — world `revision`, `stats` and a
  content checksum are unchanged by derivation; repeated requests are equal.
  `mixed_scene` also re-checks the checksum around every derivation at runtime.
- `edit_changes_aggregate_occupancy_and_rederives_both_paths` — removing the
  feature changes exactly 64 cells, drops exactly one coarse solid cell, drops
  4 exposed coarse faces (= 8 triangles), and drops 128 fine triangles
  (80 feature faces lost, 16 pedestal tops gained); the old tile does not match
  the new source stamp.
- `valid_empty_tiles_produce_no_geometry` — an all-air level-1 request is `Ok`,
  empty and produces no vertices/indices; the coarse walk skips it.
- `unsupported_level_is_reported_by_the_walk` — level 3 surfaces as an error.

Source identity: application-owned `SCENE_ID` + seed + `World::revision`
(`SourceStamp`). Every derived tile is checked with
`tile.seed()`/`tile.source_revision()` before its mesh is accepted, and an old
tile is explicitly asserted not to match a newer revision.

## Verification commands and results

Environment: `CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/coarse-terrain/target-visual`,
`CARGO_BUILD_JOBS=2`, rustc/cargo 1.96.0. Direct exit codes, no pipelines.

| # | Command | Exit | Result |
| - | ------- | ---- | ------ |
| 1 | `cargo test --locked -p matterweave-render --example terrain_tiles_smoke` | 0 | 6 passed / 0 failed (`cpu-tests.log`) |
| 2 | `cargo clippy --locked -p matterweave-render --example terrain_tiles_smoke -- -D warnings` | 0 | no findings in the example; only the pre-existing vendored-winit `function_casts_as_integer` warning outside scope (`clippy.log`) |
| 3 | `cargo build --locked -p matterweave-render --example terrain_tiles_smoke` | 0 | debug profile (`build.log`) |
| 4 | `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json MATTERWEAVE_VALIDATION=1 timeout 180 xvfb-run -a target-visual/debug/examples/terrain_tiles_smoke` | 0 | 4 frames presented; 0 bytes stderr; validation layer active (`native-stdout.log`, `native-stderr.log`) |
| 5 | same binary `--hold`, `timeout 12` | 124 | expected: kept presenting after frame 3 until killed; 0 bytes stderr (`native-hold-*.log`) |
| 6 | `rustfmt --edition 2021 --check <example>` | 0 | formatted |
| 7 | `python3 tools/check_docs.py` | 0 | docs links consistent |

Native frame evidence (CPU counts before upload, printed only after
`FrameResult::Presented`; stderr empty):

| Frame | Scene | source rev | triangles | fine | coarse | coarse tiles | coarse solid cells | face size |
| ----- | ----- | ---------- | --------- | ---- | ------ | ------------ | ------------------ | --------- |
| 0 | all-fine reference | 15184 | 12272 | 12272 | 0 | 0 | 0 | 1 |
| 1 | mixed fine-left / level-2 coarse-right | 15184 | 5512 | 5024 | 488 | 1 | 201 | 4 |
| 2 | mixed after edit | 15248 | 5504 | 5024 | 480 | 1 | 200 | 4 |
| 3 | all-fine after edit | 15248 | 12144 | 12144 | 0 | 0 | 0 | 1 |

Recorded deltas frame 1 -> 2: revision +64, coarse solid cells -1, coarse
triangles -8; frames 0 -> 3: all-fine triangles -128. Every presented frame
reported `gpu_mesh_bytes > 0`, `shadow_casters=1` and (after each upload)
`shadow_map_updated=true`.

Device/stack: Vulkan 1.4.318, `llvmpipe (LLVM 20.1.2, 256 bits)`, single
7307 MiB device-local heap, `VK_LAYER_KHRONOS_validation` enabled and active
(printed `validation true`). This is a software rasterizer under Xvfb, not
Android hardware and not a performance measurement.

## Issues and friction

- `cargo clippy --example` (binary target) initially failed with dead-code
  lints for test-only helpers. Fixed by making the runtime paths use them
  (checksum around derivation, count of duplicated LOD-plane faces after
  culling, level in the printed evidence) instead of `#[cfg(test)]` markers.
- `winit` 0.30 `KeyEvent` has `logical_key`, not `key`; Escape handling uses
  `logical_key`.
- The first clippy attempt was cut off by the session deadline; resumed and
  completed with the cached target directory.
- The provisional core files remain modified/untracked in the worktree by
  design; this worker did not stage or edit them.

## Decisions and rationale

- One aggregated `Renderer::upload` per frame. Uploading by chunk key would
  invent arbitrary keys for derived geometry and mix coarse/fine namespaces;
  aggregation keeps upload semantics and the revision guard simple.
- Non-streaming authoritative world: the example owns one world and has no
  async publication or override/residency behavior in scope.
- Level 2 for the mixed scene: a coarse face is 4 fine cells per edge, which is
  visibly and measurably distinct from the fine side; level 1 is still covered
  by the adapter contract test.
- The mixed split is a vertical cliff at `x = 0` by scene design, so the coarse
  side legitimately owns the boundary plane. Fine boundary faces that the
  authoritative neighbor occludes are removed; the assertion that none remain
  on the plane is a scene contract, not a crack-free LOD claim.
- Conservative aggregation is kept as the core defines it: thin holes may fill
  and the coarse surface may sit up to `(1 << level) - 1` fine cells above the
  fine surface. This example measures geometry, not image equality.

## What is NOT established

- No image/pixel comparison or screenshot was produced. `Presented` and draw
  counts are not visual correctness evidence; no claim is made that the frames
  look right, only that the recorded geometry contracts hold and the frames
  presented without validation messages.
- No Android build, device, emulator or APK wiring was run. Android visible
  integration remains lead-owned and is **NOT RUN** here.
- No performance, memory or thermal claim: llvmpipe/Xvfb numbers are not mobile
  evidence, and no timings are reported.
- No collision coupling: coarse tiles are display-derived only.
- Levels above 2 are unsupported by the source API and rejected by the walk.
- The seam policy (coarse side owns the split plane) is exercised only by this
  cliff scene.

## Reproduce

```sh
cd /mnt/bench/matterweave-dev/worktrees/swarm-coarse-visual
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/coarse-terrain/target-visual
export CARGO_BUILD_JOBS=2
cargo test  --locked -p matterweave-render --example terrain_tiles_smoke
cargo clippy --locked -p matterweave-render --example terrain_tiles_smoke -- -D warnings
cargo build --locked -p matterweave-render --example terrain_tiles_smoke
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json MATTERWEAVE_VALIDATION=1 \
  xvfb-run -a "$CARGO_TARGET_DIR/debug/examples/terrain_tiles_smoke"
```

Artifacts: `/mnt/bench/matterweave-dev/coarse-terrain/visual-implementation/`
(`cpu-tests.log`, `clippy.log`, `build.log`, `native-stdout.log`,
`native-stderr.log`, `native-hold-stdout.log`, `native-hold-stderr.log`).

Lead acceptance: six CPU tests and four native Vulkan phases verified in actual logs.
Gemini/high w_4f975b9b independently reviewed geometry, declared seam handling, edit
provenance and termination with no actionable findings. Timed-out runs w_355788a2 and
w_68ad6a9a produced useful checked work; completion is based on inspected evidence.
