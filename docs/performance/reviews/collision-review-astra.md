# Fresh Pi Astra collision review

1. **Scope:** Read collision implementation/tests, minimal physics hooks, and authoritative detail coordinate/transform APIs. No writes, shell, descendants, or coordination.
2. **Candidates:** None substantiated.
3. **Evidence:** `crates/matterweave-physics/src/detail_collision.rs:150,189,202,272,359,411` shows Euclidean negative-coordinate partitioning, prechecked cell/scratch/box budgets, validation before collider removal, and old/new-region waking. Yaw direction agrees with `crates/matterweave-detail/src/lib.rs:140`.
4. **Verification:** Tests **NOT RUN** under read-only. Inspected tests cover grounding, transforms, edits/restoration, overlap policy, sharing, collider-limit rejection, waking, and stationary partition seams.
5. **Uncertainty:** No runtime proof of moving seam traversal or support-removal waking. Admission caps are not peak-memory measurements: replacement temporarily retains old/new shapes, and commit still allocates. No recoverable `Err` path was found after mutation begins.
