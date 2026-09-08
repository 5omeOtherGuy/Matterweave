# collision-review-gemini

Frozen source reviewed independently; candidate findings, not acceptance.

### Findings (3)

#### F1 — `counts.instances` check rejects scenes with non-collidable instances exceeding collider limit
- **File/Line**: `crates/matterweave-physics/src/detail_collision.rs:198`
- **Trigger**: Scene with >16,384 total instances where collidable instances ≤ 16,384 (e.g., dense forest with 20,000 decorative/liquid plants within `DetailScene::MAX_INSTANCES = 200_000`).
- **Consequence**: `replace_detail_scene` aborts with `Err("...above the 16384 collider limit")`, prematurely conflating total scene instances with static colliders.
- **Evidence**: `MAX_DETAIL_COLLIDERS` doc states "static detail colliders (collidable instances) accepted"; line 245 already bounds `pending.len() >= MAX_DETAIL_COLLIDERS` after filtering collidable prototypes.
- **Uncertainty**: Affects only scenes with >16,384 total instances; NOT RUN.

#### F2 — `volume.occupied_cells()` check rejects large non-collidable prototypes
- **File/Line**: `crates/matterweave-physics/src/detail_collision.rs:108`
- **Trigger**: A decorative or liquid prototype exceeding 1,048,576 occupied cells (valid in `DetailVolume`, where `MAX_VOLUME_CELLS = 4,194,304`).
- **Consequence**: Pre-check fails with `Err("...above the 1048576 collision-cell limit")` before checking `material_policy`, blocking valid large water bodies or decorative foliage canopies that produce zero colliders.
- **Evidence**: Line 108 checks `volume.occupied_cells()` before line 116 filters `MaterialPolicy::Collision`. Doc claims sparse streaming bounds before shape construction, but cell count check precedes material filtering.
- **Uncertainty**: Trigger requires single prototype >1M occupied cells; NOT RUN.

#### F3 — Multi-prototype source cells and box limits checked after allocation
- **File/Line**: `crates/matterweave-physics/src/detail_collision.rs:215-227`
- **Trigger**: Scene with aggregate collision cells > 4,194,304 or merged boxes > 262,144 across multiple valid individual prototypes.
- **Consequence**: Breaches documented "bounded before allocation" contract: `prepare_prototype` allocates `BTreeSet`, executes `greedy_boxes`, and constructs Parry BVHs for each prototype before aggregate totals are checked and rejected.
- **Evidence**: Lines 215–227 check `source_collision_cells` and `merged_boxes` only after `prepare_prototype` returns `Ok(Some(shape))`.
- **Uncertainty**: Uncommitted shapes are safely dropped on `Err` return so physics state remains uncorrupted; NOT RUN.

---

### Engineering Log

- **Actions Taken**: Read `detail_collision.rs`, `tests/detail_collision.rs`, physics `lib.rs` hooks, `Cargo.toml`, and related `matterweave-detail` scene/volume boundaries; audited greedy box algorithm, coordinate math, quarter-turn quaternions, Rapier `ColliderSet` removal/reinsertion, and transaction boundaries. No writes/shell executed; tests NOT RUN.
- **Issues & Friction**: Tracing compound shape handling across Parry/Rapier required inspecting controller grounding and manifold generation history in execution logs.
- **Decisions & Rationale**: Prioritized boundary/contract discrepancies between `DetailScene` capacities (200k instances, 4M cells) and collision pre-checks; verified greedy cuboids preserve exact geometry and support negative coords.
- **Solutions Applied**: None; leaf read-only review.
- **Insights**: Rejection of raw `parry::shape::Voxels` in favor of greedy compound cuboids correctly addresses character controller grounding gaps; transactional safety of `self.colliders` commit is sound, but instance/prototype limits conflate collision geometry with non-collidable detail content.
