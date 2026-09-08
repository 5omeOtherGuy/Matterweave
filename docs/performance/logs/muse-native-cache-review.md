# muse-native-cache-review

## Leaf review — frozen `cc46873` (read-only, no execution)

Scope read:
- `apps/explorer/src/gallery.rs` (full, incl. tests tail)
- `apps/explorer/src/lib.rs` (Explorer + `run_desktop`/`android_main` routing + tests)
- `apps/explorer/src/dynamic_upload.rs` (full)
- `crates/matterweave-physics/src/dynamic_cache.rs` (full)
- `crates/matterweave-physics/tests/dynamic_cache.rs` (full)
- `crates/matterweave-physics/src/lib.rs` (full read for `Physics::{dynamic_mesh,step,restore,fixed_step}`, `accumulator`, `mesh_revision`, `append_box`)
- `docs/performance/p03/native-gallery.md`, `docs/performance/p02.md`
- Contract context only: `crates/matterweave-render/src/lib.rs` `upload`/`upload_dynamic`/`GpuMesh::rewrite`/`validate_mesh`, `crates/matterweave-detail/src/scene.rs` `prototype_mesh`/`draws`, `apps/explorer/src/metrics.rs` schema header.

### Findings: no concrete reachable regression candidates

No behavior/lifetime/interface/counter violation found in scope that is both reachable and contradicts the stated contracts. Reporting zero is intentional; observations below are explicitly **not** regressions.

### Observations (not regressions — hypotheses / accounting notes)

1. **Gallery budget len-vs-capacity (accounting, by-doc excluded).** `gallery.rs::combine` enforces `MAX_COMBINED_MESH_BYTES` on `len()`-projected payload before each append; `GalleryStats::combined_mesh_bytes` reports `capacity()` via `mesh_bytes()`. `Vec::extend` growth can make capacity exceed the len-checked budget. Docs explicitly exclude allocator metadata/GPU state and report capacities separately, so this is a known accounting gap, not a safety or correctness regression. Uncertainty: medium — did not measure flora capacity factor; no shell to run.
2. **Cache `mesh.revision` intentionally diverges on reuse (documented).** `DynamicMeshCache::update` only assigns `mesh.revision = physics.mesh_revision` on rebuild; `Physics::fixed_step` bumps `mesh_revision` every fixed step even for sleeping bodies. Reused cache therefore lags `Physics::dynamic_mesh().revision`. Both cache docs and `Renderer::upload_dynamic` (“revision is informational”) state revision must not be a key, and current callers use `rebuilt` bool + epoch, not revision. Future callers must preserve this. Uncertainty: low — verified by code reading only.
3. **No headless integration test for Explorer empty-clear + epoch re-upload seam.** Unit levels cover it separately: `dynamic_cache.rs` tests empty rebuild/clear, `dynamic_upload.rs` tests dirty-retention and new-epoch force-upload, `GpuMesh::rewrite(count==0)` clears `index_count`. `Explorer::sync_render_meshes` wires them correctly by reading (`needs_upload` → `upload_dynamic` → `uploaded` only on `Ok`). There is no single headless test proving the composed “failed upload on epoch N, no rebuild on N+1, still uploads; empty mesh clears” path through `Explorer` (needs GPU). Lead’s 90-frame smoke is reported context covering it. Uncertainty: medium — plausible-bug area, but no defect observed; not filing as regression.

Missing-test note (same category): gallery `combined.revision = 1` vs `Renderer::upload` guard (`r > mesh.revision → skip`) is safe because `GalleryApp` owns one fixed mesh and recreation creates a new `Renderer` (`mesh_revision: None`), with `uploaded` flag reset in both `suspended`/`resumed`. Covered by host `--gallery-exercise`, not by unit test. No defect.

### Contract checks (read, not executed)

- Routing: `run_desktop` and `android_main` resolve `gallery::Request` **before** any `World` load; `Err` exits 2 (host) / returns without world (Android). `--smoke-exercise` rejected in gallery; `--gallery-exercise` rejected without request. Malformed/oversized/unreadable marker → scoped `GalleryError`, no fall-through. Matches “must not fall through” contract.
- Gallery isolation: `GalleryView::build` touches only `DetailScene`; `GalleryApp` holds no `World`/`Physics`/save path; test `building_a_view_never_touches_user_world_data` covers valid/corrupt/missing. Combined mesh once per renderer via `ensure_uploaded` + reset on suspend/resume. Flora `validate_lod` enforced in both `parse` and `build`.
- P02 keys: `RenderBody {pose, dimensions, material}` ordered `Vec` compare; identical-pose slerp bypass; alpha from `accumulator/FIXED_DT`; includes disabled bodies (all `objects`); wake/removal/fracture/restore/dimension/material/empty all invalidate. `DynamicUploadState` separates CPU-dirty from GPU-epoch; `?` before `uploaded()` retains dirty on failure.
- Counters: Explorer `StageCapture` counts actual `rebuilt`/`upload` work; `build_ms`/`upload_ms` are `Some(0.)` when capturing and skipped, `None` when not capturing; `mesh_sync_wall_ms` wraps whole `sync_render_meshes` including compare work. Gallery zeroes/absents non-run systems per its contract.

### Actions
- Read listed scope files + renderer/detail context for upload/revision semantics.
- Compared cache keys, interpolation, disabled-body handling, empty-clear, epoch logic against contracts.
- No edits, no shell, no image review, no descendant delegation.

### Issues/Friction
- None blocking. `grep` output lacks line numbers in this harness; refs below use file + symbol instead of exact lines.

### Decisions/Rationale
- File zero regressions rather than inflate len-vs-capacity or missing-composed-test notes into actionable regressions. Both are either doc-excluded or covered by separate unit tests + reported smoke.
- Did not re-audit detail foundation/flora source; used only `prototype_mesh`/`draws`/`counts` signatures to verify adapter use.

### Solutions
- None (read-only assignment).

### Insights
- The trickiest correct-by-design points are (a) alpha-only changes must rebuild even with zero fixed steps — covered by `fractional_interpolation…` test; (b) sleeping rotated bodies must stay cached via exact `previous == current` bypass — covered by `settled_rotated_body…` test; (c) revision must never be used as a render key — current code honors this.

### Checks
- Pass/fail/not-run (honest): **not run** — this leaf executed no tests, Clippy, fmt, SHA verification, or device/host smokes. Reported context from lead (180 workspace tests, Clippy/fmt, 90-frame Vulkan smoke, host gallery sentinels, APK packaging) was **not** re-executed or independently verified here.

