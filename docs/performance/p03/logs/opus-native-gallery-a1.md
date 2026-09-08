# Session log: native detail gallery A1 (bounded leaf)

Workspace `/mnt/bench/matterweave-dev/worktrees/performance-p00`, HEAD `3581de0`
plus lead-owned uncommitted docs/tools. Deliverable:
[native-gallery.md](../native-gallery.md).

## Actions

1. Read `docs/performance/p03/foundation.md` and `verification.md` for the API
   seam, then the actual sources: `crates/matterweave-detail/src/scene.rs`,
   `fixtures.rs`, `crates/matterweave-core/src/mesh.rs`,
   `crates/matterweave-render/src/lib.rs` upload paths, `apps/explorer/src/lib.rs`
   and `controls.rs`. Did not repeat the P03 source audit or the 35 detail tests.
2. Wrote `apps/explorer/src/gallery.rs` with 9 behaviour tests first, then ran
   them against deliberately stubbed `Request::parse`, `combine` and `viewpoint`
   to confirm RED: 9 failed, 0 passed.
3. Restored the real implementation: GREEN, 9 passed.
4. Added the viewer application (window/renderer lifecycle, HUD, one-time upload,
   opt-in capture rows) and wired the opt-in into `run_desktop` and
   `android_main` before any world load. Added 3 normal-mode regression tests in
   `lib.rs`.
5. Added the `matterweave-detail` path dependency to `apps/explorer/Cargo.toml`
   and regenerated only that `Cargo.lock` edge (`cargo update --offline -p
   matterweave-explorer`, one added line).
6. Ran fmt, Clippy, package tests, workspace tests, four host gallery runs, the
   failure-path runs, the normal-mode regression smoke and the ARM64 APK build.

## Issues and solutions

- **Issue.** `--locked` refused to build after adding the path dependency.
  **Solution.** `cargo update --offline -p matterweave-explorer` added exactly one
  lock line (`matterweave-detail` under `matterweave-explorer`); no version
  changes, no network.
- **Issue.** Borrowing `&Mesh` from `prototype_mesh` (which takes `&mut self`)
  while iterating instances. **Solution.** Take the owned `draws()` vector first,
  group placements per prototype, then hold one prototype borrow per group. This
  also enforces the "fetch prototype geometry once per prototype" property that
  `mesh_builds` asserts.
- **Issue.** Clippy `dead_code` on a `data_directory` field kept "just in case".
  **Solution.** Removed it; the capture directory is only needed at construction.
- **Issue.** The first screenshot was blank: `xwd` fired after the 30-frame run
  had already exited. **Solution.** Ran the screenshot passes with
  `--smoke-frames 900` and captured at 12 s.
- **Issue.** stderr redirected outside `xvfb-run` was lost (empty log files).
  **Solution.** Redirect inside `xvfb-run -a bash -c "..."`; re-ran the normal
  smoke and the env-var gallery run to capture real output.

## Decisions

- **Separate application object, not a mode flag inside `Explorer`.** `GalleryApp`
  has no `World`, `Physics`, `save_path` or `Session`, so world/session isolation
  is structural rather than a set of runtime guards that a later edit could miss.
  Startup, recovery, action, focus, suspend and exit save paths simply do not
  exist in this object.
- **Resolve the opt-in before constructing anything.** Both entry points resolve
  the request before `Explorer::new`, so the corrupt/missing-world recovery logic
  (which renames and writes recovery files) never runs in gallery mode.
- **Combined mesh through `Renderer::upload`, not `upload_chunk`.** The chunk path
  would require inventing chunk keys for instances, which the task forbids and
  which would corrupt the renderer's chunk bookkeeping. `upload` is the
  documented whole-world compatibility path and holds one legacy mesh.
- **Presets choose a viewpoint, not a subset of the scene.** The full accepted
  scene is always built and drawn; presets only pick a camera derived from actual
  `bounds_world` values. That keeps counts honest across presets.
- **Separate `--gallery-exercise` flag.** `--smoke-exercise` asserts gameplay
  edits and saves; reusing it in a mode that must never write user data would
  either be false or force gameplay writes. The gallery flag is rejected without
  an active gallery request, and `--smoke-exercise` is rejected with one.

## Insights

- `SceneCounts::mesh_builds` is a good cheap oracle for "no per-instance
  geometry": 2 builds for 7 instances at one LOD, 6 builds across three LODs.
- Passing the environment value in as a parameter (`Request::resolve(marker,
  env)`) keeps the opt-in unit-testable without mutating process environment in
  parallel tests.
- The scene's cached-mesh bytes depend on which LODs were requested (691 KiB for
  Source only) and are not the foundation's 974,064 B all-LOD figure; reporting
  the combined mesh separately from the cache avoids conflating the two.

## Result

Implemented, verified on the host, and bounded: no flora, full-map, physics,
GPU-instancing or optimisation work was started. Device checks were not run and
are not claimed.
