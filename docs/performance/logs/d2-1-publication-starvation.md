# d2-1-publication-starvation — engineering log (2026-09-12)

Slice: settle whether the detail-collision publication gate can withhold a
prepared result **forever** because a dynamic body permanently overlaps a
prepared collider, or whether it only defers until a body genuinely leaves the
way. Base `7dace54`, branch `phase-a/d2-1-publication-starvation`.

**Verdict: REPRODUCED.** A completed structural-mode preparation was retained
and never published across 256 simulated frames while the only body/collider
overlap was the resting body against **unchanged live** floor collision. The
smallest sound in-crate fix was applied and verified.

## Publication-state note

States, using the cadence's own vocabulary. The gate withholds exactly one
transition: **completed-unpublished → published**. Queued → inflight is worker
scheduling and inflight → completed-unpublished is preparation; neither is
gated, and the gate never drops: a refused result stays buffered (worker) or
staged (workerless) and is retried.

| State | Meaning | Gate involved |
| --- | --- | --- |
| queued | `on_edit` stored an immutable source snapshot; worker has not picked it up | no |
| inflight | worker is building `PreparedDetailCollision` | no |
| completed-unpublished | preparation finished; result buffered or staged but not committed | **yes** |
| published | `Physics::publish_detail_scene` committed; gate state cleared | outcome |

Which transition each mode withholds:

- **Region mode** (`on_edit(.., Some(added))`): withholds
  completed-unpublished → published while a dynamic body AABB overlaps an
  accumulated *added-solid* region. A body resting on unchanged geometry
  overlaps no added region, so region-mode edits are not starved this way.
- **Structural mode** (`on_edit(.., None)`, or `MAX_PENDING_ADDED` overflow):
  - before the fix: withheld while any dynamic body AABB overlapped **any**
    prepared collider — including colliders that were already live and
    unchanged, which is the starvation reproduced here;
  - after the fix: withholds while any dynamic body AABB overlaps a prepared
    collider that is **not already live at the same pose with the same shape**,
    i.e. exactly while a body overlaps material the publication would add.

The note also lives as the module doc of
`crates/matterweave-physics/tests/detail_structural_starvation.rs`.

## Actions Taken

- Read the gate (`crates/matterweave-physics/src/detail_cadence.rs`),
  `detail_collision.rs`, `async_detail_collision.rs`, the frozen
  `detail_cadence.rs` suite, the prior attempt at
  `detail_structural_starvation.rs`, and the design log
  `docs/performance/logs/engine-03-collision.md`.
- Rejected the prior attempt's central fixture claim after measuring it
  (see Issues & Friction) and rebuilt the test around a dynamic voxel body.
- Wrote the reproducer + control as `tests/detail_structural_starvation.rs`
  (6 tests; worker and workerless variants of each scenario).
- Verified RED at the base source for the reproducer (test commit `379dab9`),
  then implemented the fix and verified GREEN.
- Ran the mutation probe, frozen suite, package/workspace tests, `fmt`,
  `clippy` and `check_docs.py`.

### Observed (measurements, not hypotheses)

- A 0.5 m dynamic voxel crate dropped onto the prepared 8 m x 8 m floor rests
  with ~1.2 mm contact penetration: body AABB `min.y = 0.2488` against floor
  collider AABB `max.y = 0.25`, so the gate's inclusive `Aabb::intersects`
  reads "overlapping" **permanently** while gravity holds the crate there.
- The **character** does *not* reproduce it: after `spawn_on_floor` the
  capsule AABB `min.y = 0.2671`, ~17 mm above the floor top, because
  `KinematicCharacterController` keeps its `offset` gap. The lead's candidate
  wording ("a character resting on the floor overlaps a prepared floor
  collider permanently") is false for this controller; the reproduction needs
  a rigid voxel body.
- Pre-fix run, both reproducer variants fail by starvation, both controls pass:

  ```
  $ cargo test -p matterweave-physics --locked --test detail_structural_starvation
  worker_structural_edit_clear_of_a_resting_body_publishes_immediately ... FAILED
  workerless_structural_edit_clear_of_a_resting_body_publishes_immediately ... FAILED
  worker_structural_new_material_inside_a_resting_body_defers_until_it_clears ... ok
  workerless_structural_new_material_inside_a_resting_body_defers_until_it_clears ... ok
  test result: FAILED. 2 passed; 2 failed
  ```

  Worker failure message, showing completion, retention and a clear added wall:

  ```
  structural-mode result starved for 256 simulated frames: the completed preparation
  was retained (stats AsyncDetailStats { queued: 0, inflight: 0, results: 1,
  discarded: 0, generation: 0 }) while the only prepared collider any body overlapped
  was the unchanged live floor, and the added wall is clear of every body
  ```

- An independent probe (`step_objects` each frame) confirmed the overlap was
  live on **all** 256 frames and that `discarded` stayed 0: the result was
  retained, not dropped, and live collision stayed floor-only the whole time.
- `DetailScene` at this commit has **no public instance removal or move API**
  (grep for `remove`/`move_instance`/`instances.remove` in
  `crates/matterweave-detail/src` is empty). Structural `None` therefore comes
  from host code that adds instances (the explorer passes `None` only when
  `cell_world_aabb` fails for a removed prototype) and from the
  `MAX_PENDING_ADDED` overflow latch. The defect is latent for current hosts
  but reachable through the documented contract; a structurally latched scene
  is never published while a body rests on any of its colliders.

## Issues & Friction

- The prior attempt (`detail_structural_starvation.rs`, never compiled and
  never run) asserted `body_aabbs_overlap(&physics, FLOOR_BOX)` for the
  resting character. That assertion is false: the character controller's
  offset leaves a ~17 mm AABB gap. Compiling and running it would have failed
  at the fixture. Every claim inherited from that file was re-derived; the
  character-based scenario was replaced by the voxel-crate scenario that
  actually reproduces.
- The reproduction depends on contact-solver penetration (rigid body) rather
  than controller offset (character). This is inherent, not a test artifact:
  the gate compares AABBs inclusively, and a resting rigid body straddles the
  collider AABB by a sub-millimetre contact penetration by construction.
- Test-runner stdout is swallowed by the harness here (as recorded in
  `e1-cadence-20260912.md`); probe values were read from panic messages.

## Decisions & Rationale

- **Discriminator without timing.** Completion is established structurally:
  the workerless path builds inside `on_edit` (so `Ok(true)` is
  completed-and-withheld and `Ok(false)` is already published), and the worker
  path waits on the controller's own `stats().results` counter. The stall
  bound is **256 simulated frames** (`step_objects` once per frame), not a
  sleep or wall-clock deadline; the only `Instant` is a 30 s hang guard around
  the worker-completion wait, consistent with the frozen suite. While
  withheld, every frame asserts that the body still overlaps the unchanged
  floor, that the added wall is clear of every body, and that live collision
  did not change — so "slow worker" and "correct deferral" cannot be confused
  with "stall".
- **Fix scope.** Keep the gate and the "never publish solid material inside a
  dynamic body" invariant; change only what "in the way" means for the
  structural fallback. `Physics::detail_structural_blocked` proves a prepared
  collider is unchanged by exact pose equality plus structural shape equality
  against a live collider, and ignores exactly those. It is `pub(crate)`; no
  public API changed. `PreparedDetailCollision::collider_aabbs` is kept (it is
  public) and re-documented as a diagnostic surface.
- **Pose + shape, not AABB.** An AABB-equality diff is unsound (interior fill,
  same-position prototype swap, 180-degree yaw all keep the outer AABB while
  moving material); shape/pose equality is the direction that cannot add
  material inside a body. The `same_shape` helper deliberately treats any
  shape outside this module's compound-of-cuboids family as unequal.
- **Indexed diff for cost.** Live colliders are indexed by exact canonical
  translation, so the diff is linear in collider count per call instead of
  quadratic. A structural latch can persist for many frames while a body
  genuinely blocks, and a full-map scene can hold `MAX_DETAIL_COLLIDERS`
  (16,384); an O(live x prepared) scan per frame was not acceptable.
- **No stale-merge shortcut.** Neither partial publication nor merging stale
  prepared results was considered: both are excluded by the task and
  unnecessary once the gate compares against live material.

## Solutions Applied

- `crates/matterweave-physics/src/detail_collision.rs`
  - `same_pose` (exact translation + rotation), `translation_key` (canonical
    bit key, `-0.0` folded), `same_shape` (recursive compound-of-cuboids
    equality; anything else unequal), and
    `Physics::detail_structural_blocked` (body AABB overlap with a prepared
    collider that is not live-identical, indexed by translation).
  - Doc corrections: `collider_aabbs` is diagnostic now; `detail_added_blocked`
    no longer claims the structural fallback uses coarse AABBs.
- `crates/matterweave-physics/src/detail_cadence.rs`
  - `gate_blocked`'s structural branch now calls
    `physics.detail_structural_blocked(prepared)`; module and `step` docs
    describe the new structural semantics.
- `crates/matterweave-physics/tests/detail_structural_starvation.rs` (new, 6
  tests over a shared helper each):
  - `structural_edit_clear_of_a_resting_body`: reproducer/regression — the
    clear far-wall structural edit must publish immediately (pre-fix: starved);
  - `structural_edit_inside_a_resting_body`: control — genuinely new material
    inside the body defers and publishes the instant the body clears;
  - `structural_same_aabb_replacement_inside_a_resting_body`: invariant guard —
    an added instance whose outer AABB is byte-identical to a live sparse
    instance but fills a hole inside the body still defers, then publishes.

### Mutation probe (soundness of the fix)

Temporarily replacing the predicate with an AABB-only live-vs-prepared diff
made both same-AABB guard tests fail at frame 0 (new solid material published
through the resting crate); the real predicate restored 6/6 green. The shape
comparison is load-bearing, not decorative.

## Verification

| Check | Command | Result |
| --- | --- | --- |
| Reproducer RED at base source | `cargo test -p matterweave-physics --locked --test detail_structural_starvation` | PASS (2 failed: starvation; 2 passed: controls) |
| New suite GREEN after fix | same command | PASS (6 passed) |
| Frozen regression suite untouched | `git diff --stat 7dace54 -- crates/matterweave-physics/tests/detail_cadence.rs` | PASS (empty; 11 tests pass) |
| Package tests | `cargo test -p matterweave-physics --locked` | PASS (79 passed, 2 ignored) |
| Workspace tests | `cargo test --workspace --locked` | PASS (500 passed, 0 failed, 3 ignored) |
| Format | `cargo fmt --all -- --check` | PASS |
| Lints | `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS (0 errors; 1 pre-existing vendored-winit warning, capped by Cargo) |
| Docs | `python3 tools/check_docs.py` | PASS (183 files, 581 links) |

Definition of done:

| Criterion | Result |
| --- | --- |
| New host test drives structural mode with a dynamic body permanently overlapping a prepared collider, establishes preparation completed and was withheld | PASS pre-fix (both worker paths); recorded output above |
| Test discriminates stall from deferral without sleeps/wall-clock | PASS: fixed 256 simulated frames; completion from controller counters / synchronous `Ok(true)`; every withheld frame asserts overlap only with unchanged live collision |
| Control shows publication once the blocking condition genuinely ends | PASS (2 control tests; plus the reproducer asserts immediate publication when the gate is clear) |
| Verdict recorded (REPRODUCED/DISMISSED) with evidence | PASS: REPRODUCED |
| Publication-state note names queued/inflight/completed-unpublished/published and which transitions each mode withholds | PASS: this log and the test module doc |
| REPRODUCED: smallest in-crate fix removes the stall while keeping "never publish through a body" | PASS: `pub(crate)` predicate + doc updates; mutation-verified invariant guard |
| Frozen `detail_cadence.rs` byte-identical | PASS |
| `cargo test --workspace --locked` | PASS |
| `cargo fmt`, `cargo clippy -D warnings` | PASS |
| Lead review of diff and reproduction | NOT RUN (lead-owned) |
| Graphics-integrated Android 64-piece functional run | NOT RUN (lead-owned) |

## Insights

- The gate's invariant is about **new** solid material, not about bodies
  touching static geometry. The old structural branch answered "does any body
  overlap any prepared collider", which is only a proxy; a resting rigid body
  makes that proxy permanently true. Diffing against live collision restores
  the invariant's actual meaning.
- Whether a resting body "overlaps" a static collider AABB depends on who
  settles it: the contact solver leaves a small rigid-body penetration (sign
  in favour of overlap), while the kinematic character controller leaves an
  offset gap (sign against). Tests for this gate must pick a body type that
  produces the overlap they claim.
- A structural latch can persist indefinitely by design (a body genuinely
  inside new material keeps deferring), so any work done per frame while
  latched must stay linear in scene size; an unindexed live-vs-prepared diff
  would have converted a liveness fix into a per-frame quadratic scan on the
  full-map scene.
