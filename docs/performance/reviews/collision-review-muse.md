# collision-review-muse

Frozen source reviewed independently; candidate findings, not acceptance.

Leaf read-only review — `detail_collision.rs`, `tests/detail_collision.rs`, physics `lib.rs` hooks only. Tests NOT RUN. HEAD frozen vs `c6e2317` diff not executed (no shell); review from working-tree content.

Inspected: owned `SharedShape::compound` of `cuboid`s only — no borrowed-shape lifetime, no `voxels`/heightfield/ball path retained after documented `Voxels` manifold rejection; greedy x→z→y merge over `BTreeSet` (cost ∝ occupied cells, not bbox); pre-allocation bounds checks; commit removes all `self.detail` then inserts pending; `wake_bodies_in` via loosened merged AABB; water/decorative filtered by `material_policy`, overlap-safe per-instance colliders, LOD-independent source cells.

Candidate 1 — total-instance limit over-rejects decorative scenes
- file/line: `crates/matterweave-physics/src/detail_collision.rs:~153` (`if counts.instances > MAX_DETAIL_COLLIDERS`), const doc `~49` says "collidable instances".
- trigger: scene with `>16384` total instances but few/zero collidable (e.g. 4000-plant showcase scaled up with water+fronds).
- consequence: `Err` "collider limit" though real collider/shape cost near zero; load/edit rejected instead of accepted.
- evidence: second per-collider guard (`pending.len() >= MAX`) already bounds actual colliders; test `liquid_and_decorative` asserts 65 instances → 0 colliders, so the two counters deliberately differ.
- uncertainty: `counts.instances` semantics assumed total; may be intentional conservative cap — confirm intent, else check collidable count.

Candidate 2 — `i32` increment overflow in greedy merge
- file/line: `detail_collision.rs:~greedy_boxes` (`let mut end_x = x + 1; … end_z/end_y` loops, `remaining.remove` triple loop).
- trigger: prototype containing a cell at/near `i32::MAX` on any axis with contiguous run to the boundary.
- consequence: debug overflow panic; release wrap yields wrong boxes or runaway merge — violates loud-transactional guarantee at commit-adjacent code.
- evidence: no coordinate bound checked before `x+1`/`+=1`; scale-validity and `occupied_cells` caps count, not coordinate magnitude.
- uncertainty: HIGH — `DetailVolume::set` likely rejects such coords (detail crate out of scope, not read); no such test; treat as hardening (`checked_add`/explicit coord bound) only if reachable.

No third candidate: wake uses merged old+new AABB loosened 0.5 m (over-wakes distant moves, no under-wake found); `expanded_collision_cells` (20 M-class full scene) is report-only, storage stays per-prototype shared shapes; `self.colliders[handle]` post-insert indexing is infallible in this pin.

