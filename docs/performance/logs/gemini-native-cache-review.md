# gemini-native-cache-review

### Engineering Log & Review Findings

**Reviewer Role:** Independent READ-ONLY Pi leaf reviewer.  
**Inspected Scope:** `cc46873` across `apps/explorer/src/gallery.rs`, `apps/explorer/src/lib.rs`, `apps/explorer/src/dynamic_upload.rs`, `crates/matterweave-physics/src/dynamic_cache.rs`, `crates/matterweave-physics/tests/dynamic_cache.rs`, and `crates/matterweave-physics/src/lib.rs` (module export only).  
**Context Docs:** `docs/performance/p03/native-gallery.md`, `docs/performance/p02.md`.  
**Execution Boundary:** Static source code inspection only. No local execution, edits, shell commands, or device runs were performed by this reviewer.

---

### Reachable Regression Candidates & Behavioral Findings

#### Candidate 1: `apps/explorer/src/gallery.rs:807, 828` — Gallery Frame Capture Repurposes Dynamic Mesh Metrics and Omits Zero Timing on Skipped Frames
* **File/Line:** `apps/explorer/src/gallery.rs:807` and `apps/explorer/src/gallery.rs:828`
* **Trigger:** Running the opt-in detail gallery with frame capture enabled (`profile-frames.txt` present in data directory).
* **Consequence:**
  1. `dynamic_mesh_uploads` is populated as `Some(u32::from(self.frame_upload_ms.is_some()))` and `dynamic_upload_wall_ms` is populated as `self.frame_upload_ms`. On Frame 1, when the whole-world compatibility mesh (`renderer.upload`) is uploaded, `dynamic_mesh_uploads` emits `1` and `dynamic_upload_wall_ms` emits the upload duration. This conflates the whole-world compatibility upload with dynamic object uploads, reporting a dynamic upload even though no `matterweave-physics` dynamic bodies exist and `dynamic_mesh_builds` is `Some(0)`.
  2. On subsequent captured frames (Frames 2..20), `self.ensure_uploaded()` returns `Ok(None)` because the mesh is already resident. `self.frame_upload_ms` is therefore `None`. This causes `dynamic_upload_wall_ms` to emit `None` (written as an empty column `""` in CSV) while `dynamic_mesh_uploads` emits `0`. This diverges from the contract ("skipped timings be 0 during capture") and contrasts with `apps/explorer/src/lib.rs:943` where skipped dynamic uploads during capture emit `Some(0.0)` (`0.0000` in CSV).
* **Evidence / Repro Suggestion:** Create `profile-frames.txt` and `detail-gallery.txt` (`tile source`). Run `matterweave-explorer --smoke-frames 20`. In the generated `frame-profile-v2-*.csv`, observe row 1 has `dynamic_mesh_uploads = 1` and `dynamic_upload_wall_ms = <ms>`, while rows 2..20 have `dynamic_mesh_uploads = 0` and `dynamic_upload_wall_ms = ` (blank, not `0.0000`).
* **Uncertainty:** Low on code mechanism and CSV formatting; medium on whether external automated CSV analysis pipelines treat blank vs `0.0000` as breaking.

#### Observation 1: `apps/explorer/src/gallery.rs:184-188` — Trailing Non-Empty Lines in Request Text Silently Ignored
* **File/Line:** `apps/explorer/src/gallery.rs:184-188`
* **Trigger:** A `detail-gallery.txt` marker containing multiple lines of commands (e.g. line 1: `tile source`, line 2: `flora source`).
* **Consequence:** The parser uses `.find(|line| !line.is_empty())`, taking only the first non-empty line and ignoring subsequent lines without returning a syntax error. While extra tokens on the *same* line are strictly rejected with `expected <preset> [lod] only`, extra lines below the first command are silently dropped.
* **Evidence / Repro Suggestion:** Write `tile source\nflora source\n` into `detail-gallery.txt`. `Request::parse` succeeds and returns `Preset::Tile` instead of rejecting the multiline configuration.
* **Uncertainty:** Low. Minor parser leniency; does not compromise data safety or crash the process.

---

### Verification of Mandatory Contracts

1. **Default Game / Save / Lifecycle Behavior Preserved:**  
   *Verified.* `apps/explorer/src/lib.rs` preserves all gameplay paths, autosaves, manual saves, and object simulation. Normal-mode Vulkan 90-frame smoke test passed by lead; code review confirms standard `Explorer` execution branch remains untouched when no gallery request is present.

2. **Gallery Opt-in Isolation & Malformed Request Rejection:**  
   *Verified.* In both `run_desktop` (`apps/explorer/src/lib.rs:1445`) and `android_main` (`apps/explorer/src/lib.rs:1533`), `gallery::Request::resolve` is executed before any world is loaded or save path opened. If the marker/env request fails validation or exceeds 128 bytes, `run_desktop` exits with code 2, and `android_main` logs and aborts startup immediately. No world is loaded, created, overwritten, or recovered on malformed requests.

3. **Gallery World / Physics / Save Isolation:**  
   *Verified.* `GalleryApp` owns no `World`, no `Physics`, no `save_path`, and no `Session`. Its `suspended` and `exiting` handlers only drop windows, renderers, and flush profiling. It cannot write user save data.

4. **Cached Combined Mesh Rendering (Not GPU Instancing):**  
   *Verified.* `gallery::combine` takes prototype meshes and instance transforms, transforms vertices into world coordinates using `point_to_world` and `direction_to_world`, and combines them into a single `Mesh`. `ensure_uploaded` uploads this mesh once per renderer instance via `renderer.upload`. Recreating the renderer (e.g. during resume or `--gallery-exercise` stage 2) re-uploads the retained CPU mesh once without re-meshing.

5. **Flora Scene SourceLOD Restrictions & Canonical Seed:**  
   *Verified.* Direct construction and request parsing both enforce `validate_lod`, rejecting `Lod::Half` and `Lod::Quarter` for `Preset::Flora` with an explicit error. `Preset::Flora.seed()` returns canonical seed `20260908` (`FLORA_CANONICAL_SEED`), while tile/parasol presets return `2026` (`GALLERY_SEED`). Seed provenance displayed on the HUD dynamically tracks `view.request.preset.seed()`.

6. **P02 Cache Invalidation & Interpolation Integrity:**  
   *Verified.* `DynamicMeshCache` keys ordered tuples of `(Pose, dimensions, material)`. Exact pose equality (`object.previous == *current`) correctly bypasses `slerp` to avoid floating-point drift on sleeping bodies. Fractional interpolation between fixed steps computes new poses and invalidates `self.previous == self.next`. Sleeping rotated bodies stay cached across variable fractional steps. Support removal, fractures, and dimension/material edits invalidate the cache and rebuild geometry. Disabled bodies remain in `physics.objects` and retain visibility.

7. **P02 CPU Geometry Cache vs GPU Residency Separation:**  
   *Verified.* `DynamicUploadState` tracks `dirty` and `uploaded_epoch`. Failed uploads retain `dirty = true` without requiring CPU re-meshing on subsequent frames. Renderer recreation increments `renderer_epoch`, forcing an upload of cached CPU geometry even when physics objects are stationary (`rebuilt == false`). Empty physics objects produce an empty `Mesh`, which uploads and invokes `GpuMesh::rewrite` to clear live GPU drawing indices.

8. **P02 Capture Accounting & Sync Timings:**  
   *Verified in `lib.rs`.* In `apps/explorer/src/lib.rs:934-958`, during active capture, `dynamic_mesh_build_ms` and `dynamic_upload_ms` report `Some(0.0)` when skipped and measured wall times when executed. `mesh_sync_wall_ms` encloses the entire `sync_render_meshes` block, properly capturing the cache comparison and pose construction overhead.

---

### Standard Final Sections

* **Actions:**  
  * Completed read-only code review of `apps/explorer/src/gallery.rs`, `apps/explorer/src/lib.rs`, `apps/explorer/src/dynamic_upload.rs`, `crates/matterweave-physics/src/dynamic_cache.rs`, `crates/matterweave-physics/tests/dynamic_cache.rs`, and surrounding interfaces.
  * Verified contract conformance across lifecycle isolation, request parsing, flora restrictions, camera viewpoints, combined meshing, P02 cache invalidation, and GPU residency state machines.
  * Formulated one concrete reachable metric recording finding in `gallery.rs` and one parser observation.

* **Issues/Friction:**  
  * In `apps/explorer/src/gallery.rs:807, 828`, `GalleryApp` sets `dynamic_mesh_uploads` to 1 on frame 1 to record the static whole-world combined mesh upload, but leaves `dynamic_upload_wall_ms` as `None` on subsequent skipped frames instead of `Some(0.0)`.

* **Decisions/Rationale:**  
  * Did not classify known limitations (opaque water, sparse parasol underside framing partial occlusion) as bugs, per explicit contractual guidance.
  * Preserved strict read-only leaf reviewer boundary: avoided running unverified commands or modifying code.

* **Solutions:**  
  * None proposed or implemented (strictly read-only assignment).

* **Insights:**  
  * The separation of `DynamicUploadState` from `DynamicMeshCache` cleanly decouples CPU meshing from Vulkan surface lifecycle. Renderer recreations (epoch bumps) correctly push retained dynamic geometry to the new device context without triggering redundant CPU vertex transformations.
  * The fast-path stationary pose check (`object.previous == *current`) in `dynamic_cache.rs` successfully avoids slerp numerical noise on sleeping bodies, allowing exact float equality on `Pose` across frames with varying accumulator fractions.

* **Checks (Honest Status):**  
  * Read-only structural, contractual, and interface verification: **PASS**
  * Cargo tests / Clippy / Formatting / Vulkan smoke: **NOT RUN** by this reviewer (reported as run and passing by session lead context: 180 workspace tests, 45 explorer tests, 90-frame host Vulkan smoke).
  * Device / Phone execution: **NOT RUN** (remains pending device qualification).
