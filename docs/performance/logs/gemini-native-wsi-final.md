# gemini-native-wsi-final

### Actions
- Inspected the changes between `cc46873` and `6fac9a5` across:
  - `crates/matterweave-render/src/lib.rs` (`classify_present`, `present_results_map_to_honest_frame_outcomes`, and caller lifecycle in `draw_frame`/`draw`);
  - `apps/explorer/src/gallery.rs` (`Request::parse`, `Request::resolve`, `ensure_uploaded`, `FrameRow` dynamic metrics, and removal of obsolete `frame_upload_ms`);
  - `tools/performance/check_gallery_capture.py` (Vulkan gallery capture acceptance check).
- Evaluated Vulkan WSI swapchain recreation deferral against Khronos `VkResult` semantics, Android compositor rotation behavior (`pre_transform: IDENTITY`), semaphore synchronization, submission identity ordering, and window resize propagation.
- Traced `Request::parse` line/comment filtering, whitespace tokenization, single-directive enforcement, and LOD constraints.
- Verified schema v2 contract compliance in `validate_frame_profile.py` against gallery capture rows reporting absent dynamic pipeline timings (`None` -> `""`) and explicit zero counters (`Some(0)` -> `"0"`).

---

### Issues / Friction
- **No concrete reachable defects found** in the candidate diffs (`cc46873..6fac9a5`).
- *Observation (Dormant Field)*: In `crates/matterweave-render/src/lib.rs:1005`, `PresentOutcome::Presented { recreate: bool }` retains a `recreate` boolean field, but `classify_present` unconditionally yields `recreate: false` on `Ok(_)`, and `_acquire_suboptimal` is ignored. This is harmless, maintains enum flexibility, and correctly prevents per-frame swapchain recreation on Android advisory `VK_SUBOPTIMAL_KHR`, but leaves `recreate` functionally static.
- *Observation (Per-frame Micro-timing)*: In `apps/explorer/src/gallery.rs:739`, `mesh_sync_wall_ms` wraps `self.ensure_uploaded()`. On frame 1 this records the multi-millisecond GPU upload; on subsequent frames it measures the microsecond check of `self.uploaded == true`. This correctly distinguishes an active stage doing 0 work (`Some(ms)`) from a non-existent dynamic pipeline (`None`), passing `check_gallery_capture.py`'s `any(mesh_sync > 0)` assertion.

---

### Decisions / Rationale
- **WSI Suboptimal Recreation Deferral (`classify_present`)**:
  - `VK_SUBOPTIMAL_KHR` from `vkAcquireNextImageKHR` or `vkQueuePresentKHR` guarantees successful image acquisition and presentation. On Android devices presenting with `pre_transform: IDENTITY` when the display is rotated, Qualcomm/Adreno drivers persistently return `VK_SUBOPTIMAL_KHR`.
  - Rebuilding the swapchain on advisory suboptimal results recreates the exact same swapchain with `IDENTITY`, causing an infinite teardown/recreation loop every frame (~16 ms penalty).
  - Deferring recreation to explicit `WindowEvent::Resized` events (calling `Renderer::resize`) or `VK_ERROR_OUT_OF_DATE_KHR` (where presentation actually fails) is sound, eliminates pipeline rebuild loops, and preserves correct compositor rotation.
- **Synchronization & Semaphore Safety**:
  - If `queue_present` returns `VK_ERROR_OUT_OF_DATE_KHR`, the command buffer submission has already been queued to the device queue with `self.commands.fence`.
  - On the subsequent retry frame, `Renderer::draw` calls `self.commands.wait()` *before* checking `self.recreate`. Once `commands.wait()` clears, `device_wait_idle()` is called prior to swapchain teardown.
  - The `self.commands.available` semaphore is properly consumed by `queue_submit`, and image-specific `s.finished` semaphores are dropped only after the GPU is completely idle. Submission IDs increment monotonically and are recorded causality-cleanly with timestamp queries.
- **Gallery Metric Semantics**:
  - Because the detail gallery contains no dynamic physics/voxel bodies, reporting `dynamic_mesh_build_wall_ms: None` and `dynamic_upload_wall_ms: None` is honest and matches schema v2 optional duration rules.
  - Setting dynamic counts (`dynamic_mesh_builds`, `dynamic_mesh_uploads`, `voxel_bodies_*`, `save_attempts`, `save_failures`) to `Some(0)` satisfies balance and non-negative integer schema validation.
- **Request Parsing Robustness**:
  - In `apps/explorer/src/gallery.rs:159-168`, `text.lines().map(|line| line.split('#').next().unwrap_or("").trim()).filter(|line| !line.is_empty())` strips inline comments and skips blank/comment-only lines. Calling `lines.next()` once for the directive and checking `lines.next().is_some()` correctly errors on multiline directives while cleanly tolerating leading, trailing, and inter-line comments.

---

### Solutions
- None (Candidate diff is sound as written; no modifications required).

---

### Insights
- Distinguishing between "unimplemented/absent subsystem" (`None` / empty cell) and "executed subsystem with 0 work" (`Some(0.)` / `Some(0)`) preserves metric integrity without compromising automated validators like `validate_frame_profile.py`.
- Moving the one-time static mesh upload timing into `mesh_sync_wall_ms` avoids polluting dynamic upload telemetry while preserving visibility into initial asset upload latency.

---

### Honest Checks Not-Run
- Did not execute `cargo test`, `cargo clippy`, or `cargo build`.
- Did not run host acceptance script `python3 tools/performance/check_gallery_capture.py` or `xvfb-run`.
- Did not execute on physical Android hardware or emulator.
- Did not verify git commit object SHAs or cryptographic file digests.
- Did not review prior Muse or Gemini audit logs.
