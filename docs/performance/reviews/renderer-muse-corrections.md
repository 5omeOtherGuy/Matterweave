# renderer-muse corrected renderer review

## Engineering Log — read-only leaf review `performance-p00`

Scope frozen: `crates/matterweave-render/src/lib.rs`, `crates/matterweave-render/src/static_scene.rs` (+ `world.wgsl:95-106` for wetland opt-in). No edits/shell/tests/device. Lead concurrently edits `app/docs` only.

Verified statically:

* Shared pooled buffers: `static_scene.rs:StaticScene::new` creates one `vertices` (`VERTEX_BUFFER`), one `indices` (`INDEX_BUFFER`), one packed `instances` (`VERTEX_BUFFER`, bound at `binding=1, INPUT_RATE=INSTANCE` in `lib.rs:pipeline`). `record_batches` binds `0/1 + index buffer` once, then one `cmd_draw_indexed` per `PrototypeRange`.
* Per-instance ranges: `plan_static_scene` groups by prototype, `instance_offset=instance_bytes(packed_so_far)` = instance-index count, emitted as `firstInstance` in `cmd_draw_indexed(index_count, instance_count, first_index, vertex_offset, instance_offset)`. `first_index/vertex_offset` accumulate across merged prototypes with `u32/i32` checked conversions. Correct index- vs instance-domain units.
* `lib.rs:1372 replace_static_scene`: host plan first, `commands.wait()`, `instance_count==0 → None` clear, transactional `StaticScene::new` before `self.static_scene=Some`, old buffers dropped after fence wait. `update_counters` includes `static_scene.allocated_bytes`.
* `lib.rs:1423 set_world_visible`: bool only, retains all GPU resources; `draw` gates `effective.shadows`, chunk/legacy/dynamic loops, static `bounds.push`, `visible_chunks` filter, `record_batches`, HUD still submits. Legacy/chunk paths preserved.
* Wetland opt-in: `lib.rs:1429 set_wetland_material_time(Option<f32>)` validates finite, wraps `rem_euclid(TAU*10)`; `None→eye.w=1.0`, `Some→-1-t` at `lib.rs:~1700`. `world.wgsl:95 enhanced=eye.w<0`, water-only + grain/highlight under `if enhanced`; legacy path (`eye.w==1`) keeps original lighting/fog. No geometry/collision change.
* Index-buffer correction independently verified: indices buffer created with `INDEX_BUFFER` (not `VERTEX_BUFFER`), bound with `cmd_bind_index_buffer(...,UINT32)`, drawn with `cmd_draw_indexed`. No `cmd_draw` on indexed static path.

### Candidate findings (5-field)

| File/Line | Trigger | Evidence | Consequence | Uncertainty |
|---|---|---|---|---|
| — | — | — | — | — |

**Result: 0 valid findings.** No concrete defect meeting bar (exact line + trigger + evidence + consequence) found. Hashes `6a5a70…` / `885415…` not re-hashed (no shell per task scope); content reviewed as retrieved.


Lead qualification: source inspection only. Strong assurances in reviewer prose are not runtime evidence; native validation and phone tests remain separate gates. File digests identify frozen contents, not commits.
