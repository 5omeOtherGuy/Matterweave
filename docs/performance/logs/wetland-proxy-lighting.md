# Engineering log wetland-proxy-lighting

Worker slice: production wetland consumption of the MeshProxy indirect/reflection
publication (issue #47). Base `origin/main` @ `8183540` (includes the merged
interaction fix49 `bb68e59`). No merge: independent review and the Android
functional gate are the lead's rows and are not claimed here.

Owned writes: new `apps/explorer/src/wetland_lighting.rs`, `apps/explorer/src/lib.rs`
(one `mod` line), `apps/explorer/src/wetland.rs` (the `Runtime` field block, the
`WetlandDetail::installed` accessor, the detail-install arm and the dynamic-upload
tail of the frame path, plus the test `Runtime` fixture), this log. Nothing else
was changed: `crates/matterweave-render/**`, `world.wgsl`, `async_indirect.rs`,
`mesh_lighting_check.rs`, `detail_check.rs`, the action-ray path in
`wetland.rs` (`~:1113-1142`) and every other `wetland.rs` region are untouched
(`git diff --stat`: 1 + 36 lines outside the new file).

## Actions Taken

- Read `AGENTS.md`, `docs/HANDOFF.md`, `docs/STATUS.md`, the renderer contracts
  (`crates/matterweave-render/src/indirect.rs`, `reflection.rs`, `shadow.rs`,
  `lib.rs:1390-1650`) and the actual wetland call graph
  (`apps/explorer/src/wetland.rs`, `detail_runtime.rs`, `dynamic_upload.rs`,
  `apps/explorer/src/mesh_lighting_check.rs`) before writing code. Confirmed from
  source, not from the review brief: production installs geometry only
  (`wetland.rs:1503-1540`), the wetland `World` is `World::new(SEED)` and never
  populated (`:90`, `:495`), and every `MeshProxy` caller was a diagnostic.
- Implemented `apps/explorer/src/wetland_lighting.rs`:
  - `WetlandLighting::new(clearing)` — one authored coverage box, 20x10x20 m
    (`BOX_HALF_EXTENT = [10,5,10]`, 4000 cells = the indirect face-residency cap),
    anchored on the destruction-clearing landmark.
  - `update(sink, FrameSource, InstallState)` — the frame state machine; the
    production sink is `impl Publication for Renderer`, so `upload_indirect` /
    `upload_reflection` / `disable_*` / `indirect_enabled` / `reflection_enabled`
    are called through one seam that tests drive with a recording double.
  - Box-local proxy build: installed instances whose world AABB meets the box
    (AABB cache keyed by pool index + mesh revision), those meshes copied into a
    compact pool, plus the merged `DynamicMeshCache::mesh()` as one identity
    placement; `MeshProxy::build` remains the only voxelizer.
  - Materials: dominant detail-catalogue drawn colour per pool entry (exact
    reverse palette lookup); the merged physics mesh keeps its own drawn colour
    under reserved palette id 255 (the volume is rebuilt if that entry changes).
  - Digest gate: an install whose proxy footprint is unchanged discards the fresh
    proxy and keeps the cached volume, republishing it immediately; a changed
    footprint withdraws both publications and recomputes.
  - Fixed `UpdateBudget { rays: 1024, work: 8192 }`, 16 hemisphere samples; the
    volume is advanced only while `!valid_for(world, epoch, sun)` and published
    only when `valid_for` holds (complete *and* current key).
  - Containment: any build/update/upload error withdraws both, is reported once,
    and a failed build is not retried until the source identity moves.
  - A frame whose install is unavailable (`InstallState::Unavailable`) is a
    lighting no-op: installs are transactional, so the previous publication still
    describes the resident scene and is not withdrawn, and nothing is built or
    published from a selection the renderer has not accepted.
- Wired the frame path: the install arm records
  `Unavailable`/`Current`/`Installed`, and the scheduler runs after the dynamic
  upload, because every renderer geometry path and a sun change retire the
  previous publications.
- Registration: one `mod wetland_lighting;` line in `lib.rs`.

## Verification (all run in this checkout)

Focused and package tests, `CARGO_TARGET_DIR=../wetland-lighting-target`,
`RUSTC_WRAPPER=` (no inherited sccache), `CARGO_BUILD_JOBS=1`:

- `cargo test --locked -p matterweave-explorer --lib wetland_lighting`
  -> `13 passed; 0 failed; 183 filtered out`. The tests pin, in order of the
  dispatch's list: install gating and ordering (an unavailable install publishes
  nothing and leaves the last accepted publication in place; the first accepted
  install cannot publish a partial volume); unchanged
  selection reuse (no rebuild, no redundant upload, no withdrawal); digest
  gating (a reinstall of an identical footprint republishes from cache with
  `pending_work == 0`; a sub-cell body move keeps the digest while a cell move
  retires it); proxy-only world coverage (an empty authoritative world, nonzero
  proxy cells, a receiver face lit only through the proxy, the world-only control
  volume black on all six faces, water mirror > 0); failure containment (a forced
  upload rejection withdraws both, the session continues, the still-complete
  volume republishes when the transient clears; an unrepresentable mesh is
  reported once); sun change (cache retired, recomputed, republished); the box
  bounded by both engine caps and instances outside it filtered out; the
  catalogue palette complete and injective with the drift guard.
- `cargo test --locked -p matterweave-explorer --lib` -> `195 passed; 0 failed;
  1 ignored` (all pre-existing wetland/detail/pacing/gallery tests still pass).
- `cargo test --locked -p matterweave-render` -> `107 passed; 0 failed`
  (the available CPU renderer suites; no GPU-requiring test is pretended).
- `cargo clippy --locked -p matterweave-explorer --all-targets -- -D warnings`
  -> exit `0` (only the pre-existing `vendor/winit` notice).
- `rustfmt --edition 2021 --check apps/explorer/src/wetland_lighting.rs
  apps/explorer/src/wetland.rs apps/explorer/src/lib.rs` -> exit `0`.
- `python3 tools/check_docs.py` -> `PASS: 219 Markdown files, 655 local links,
  16 ADRs and 20 requirements`, exit `0`.

Host functional runs (software Vulkan `llvmpipe (LLVM 20.1.2, 256 bits)`,
Vulkan 1.4.318, `xvfb-run -a`, **host evidence only — not a device, timing or
thermal claim**):

- `--showcase --smoke-frames 90 --save /tmp/wpl-smoke2/world.json` -> exit `0`
  (final source): `WETLAND LOADED: 34716467 cells / 8302 placed objects`, then
  `WETLAND PROXY LIGHTING: gi=false reflection=true cells=1793 pool_meshes=39
  digest=Some(17984592732044421731) pending=381906 rebuild_ms=49.40` and
  `... gi=true reflection=true ... pending=0`, then `WETLAND SMOKE PASS: 90
  frames`. The production path converges and publishes on the real renderer, and
  the digest is reproducible across runs.
- Ground-route walk evidence, **pre-refactor source** (`--showcase
  --smoke-frames 600`, log `/tmp/wpl-replay/run.log`, replay walked 78 route
  waypoints from spawn `[46.0, 14.95, 66.0]` to `[75.47, 13.52, 64.09]`, exit
  `0`, wall 15m00s on llvmpipe, five transition lines): converge at the spawn
  (`gi=true`, digest `17984592732044421731`), then two camera-driven footprint
  changes (`cells 1793 -> 1801`, digest `3274210954479869833`, `rebuild_ms=54.33`;
  then `cells 1805`, digest `5186674116141439325`), and `gi=true ... pending=0`
  again — LOD/camera churn retires and then reconverges instead of staying off.
  The replay diagnostic itself ended `FAIL / total wall timeout` (its own 600 s
  guard, pre-existing).
- The same replay re-run on the **frozen source** (`/tmp/wpl-replay3/run.log`)
  stopped before walking: the replay diagnostic's pre-existing settle gate
  (`apps/explorer/src/wetland_replay.rs:122`, `!settled && !grounded && wall >=
  3.`) saw one 3.72 s llvmpipe frame and finished `FAIL / not grounded after
  settle`. No lighting transition was involved, and no walk/LOD evidence is
  claimed from that attempt; the code delta between the two replays is confined
  to the `Unavailable`/error-reporting paths covered by tests. Walk/LOD
  convergence on the frozen source is the Android worker's gate.
- Measured on this host, for Phase B input only: a proxy rebuild costs 48-54 ms
  in the debug profile on llvmpipe (first build cold AABB cache, subsequent
  rebuilds while walking warm), and the box rebuild covers 39 resident meshes /
  ~1800 occupied cells.

## Issues & Friction

- `Mesh` is deliberately not `Clone` (`crates/matterweave-core/src/mesh.rs:13`),
  so the box-local pool copies vertices/indices explicitly (`copy_mesh`); the
  first compile caught the assumption.
- **Trap found by a test, not by review:** gating the volume advance on
  `complete()` starves a key change. `IndirectVolume::complete()` only reports
  cursor position, so a volume that finished for the *old* sun reports complete
  forever and never restarts. `update` must be called whenever
  `valid_for(world, epoch, sun)` is false, which restarts the cleared key. The
  sun-change test fails against the `complete()` gate.
- Reflection is a CPU bake, not a converged cache, so it publishes in the same
  frame its proxy is attached while indirect is still accumulating. Two tests
  initially asserted "both off after a digest change"; the code is right and the
  expectations were corrected, which also documents the real ordering.
- A bare `--smoke-frames N` run drives the sandbox `Explorer`, not the wetland
  (`lib.rs:1643`); the showcase chooser needs `--showcase`. The first smoke run
  produced `SMOKE PASS ... world revision 42903` and no wetland line, which is
  exactly that mode selection, not a lighting failure.
- The 600-frame llvmpipe replay costs 9-15 minutes of wall clock (llvmpipe renders
  the 8302-instance scene at roughly 1-1.5 s/frame). Long host runs are budgeted
  for, not hung. The replay diagnostic's settle gate is wall-clock sensitive: a
  single slow frame with `frame_dt >= 3 s` fails the run before it walks, which
  is why the frozen-source replay attempt reports no walk.
- `clippy -D warnings` rejected `Vec<Option<(u64, [f32;3], [f32;3])>>`
  (`type_complexity`); the bounds pair is now a named `Bounds` alias.

## Decisions & Rationale

- **Fixed authored box at the destruction clearing** instead of a camera-following
  volume. The clearing is a flat ground-route landmark where the physics bodies
  rest, so a real body move, throw or edit lands inside the published region, and
  the box never moves — camera churn then only arrives through the installed
  detail selection, which the digest gate absorbs. A travelling or tiled
  multi-volume scheme is future work; its cost ranking is Phase B.
- **Box-local compact pool rather than the whole resident pool.** `MeshGeometry`
  takes one `&[Mesh]`, and the wetland's resident pool (893 prototypes) must not
  be copied or voxelized per rebuild. Filtering instances by world AABB and
  copying only the box's meshes bounds a rebuild by the box; it is also the only
  way the merged physics mesh can join a pool the app does not own.
- **Digest gating over selection gating.** The proxy identity is the rasterised
  cell footprint, so a reinstall, a LOD swap that keeps the cells, or a sub-cell
  body motion republishes the cached volume instead of restarting convergence.
  That is what keeps ordinary camera-driven install churn from starving GI.
- **Dynamic geometry included.** The engine documents a world-space one-entry
  pool for moving objects, and at the clearing the physics bodies are a visible
  part of the scene, so omitting them would be a coverage hole in the one region
  this slice lights. `Mesh::revision` cannot see interpolated motion, so the
  merged mesh is identified by hashing its drawn positions and topology.
- **Palette-reserved dynamic material.** The merged physics mesh carries
  physics-side colours, not catalogue ids; reserving id 255 keeps the reverse
  lookup unambiguous and lets the reflection/indirect palettes carry the colour
  that is actually drawn. A colour change rebuilds the volume, because
  `IndirectVolume`'s palette is immutable after construction.
- **Mirror strengths restricted to the two surfaces this scene draws wet**
  (water 0.8, bank stone 0.25). Reflection is opt-in and every other material
  stays nonreflective; these are palette constants for an artist to tune, not
  engine or gameplay rules.
- **A rejected install is a lighting no-op.** Renderer installs validate before
  touching live state and retain the previous scene on failure, so the previous
  publication still describes the resident geometry; withdrawing it would only
  flicker GI on a transient detail failure. The frame does no lighting work, and
  the retried install on the next frame carries the new source identity.
- **Build failure withholds, upload failure retries.** A failed build means the
  proxy no longer describes the renderer's geometry, so nothing from the
  superseded representation may publish again until the source identity moves;
  a failed upload is transient and is retried from the still-complete cache.

## Solutions Applied

- Proxy publication ordering: the scheduler is called after
  `replace_static_scene` / `update_static_instances` / `upload_dynamic` in the
  same frame, and `InstallState::Unavailable` withdraws instead of publishing.
- Convergence without starvation: digest-gated rebuild plus republish-from-cache
  when the renderer retires a publication (`pending_work == 0` in the reinstall
  test), with a fixed per-frame budget for the only case that must recompute.
- Failure containment: one `update` wrapper withdraws both publications on every
  error path, reports a repeated failure once, and the frame path only logs and
  sets `self.status` — the session keeps running.
- Honest coverage wording: the module documents the local box, the one-material
  per pool entry approximation, the unrepresented per-body materials, and the
  cell-resolution identity; no full-GI claim is made anywhere in code, tests or
  this log.

## Insights

- The renderer's publication entries are validity-flag based, so a green
  `indirect_enabled()` is not visibility; both this module and its tests treat the
  flags as contract state and leave pixel quality to Phase B.
- The mesh-proxy contract is per-*pool-entry* materials, not per-prototype:
  an app that wants more than one material per prototype must split meshes, which
  is why the one-material approximation is stated at the API level rather than
  worked around locally.
- `upload_dynamic` disabling indirect (`lib.rs:1390-1404`) makes the scheduler's
  position in the frame load-bearing; a scheduler call placed with the detail
  install would republish and then be invalidated by the same frame's dynamic
  upload.
- The renderer's geometry installs being transactional is what makes
  `InstallState::Unavailable` a safe no-op rather than a forced withdrawal;
  renderer re-creation instead shows up as `indirect_live() == false`, which the
  same state machine answers by rebuilding and republishing.

## Proposed shared docs updates (not applied; log only, per dispatch)

- `docs/STATUS.md`: add to the lighting row — "production wetland consumes the
  MeshProxy publication path: bounded 20x10x20 m GI/reflection box at the
  destruction clearing, proxy built from the installed detail selection +
  resident pool + merged dynamic mesh, digest-gated rebuilds, fixed per-frame
  work budget, errors disable both and the session continues. Host evidence only;
  device functional gate and Phase B cost/thermal remain open."
- `docs/ROADMAP.md` / `docs/PERFORMANCE_TASKS.md`: Phase B items for this feature
  are proxy-build cost per rebuild (host debug measured 48-54 ms; device release
  unmeasured), `UpdateBudget`/`SAMPLES` tuning, coverage-box size ranking,
  camera-following or tiled volumes, per-body materials, and GI pixel quality.
- `docs/HANDOFF.md`: next step is the Android functional pass (stationary
  converge, walk/LOD converge, one break/place republish, body-move republish,
  lifecycle), then independent review of the frozen SHA.
- No ADR is proposed: this is consumption of accepted engine APIs
  (ADR-0014/ADR-0015 scope), not a new architectural decision.

## Open gates (not run, not claimed)

Independent review of the frozen source, the Android functional gate, and every
Phase B cost/thermal measurement. This log makes no device or performance claim.
