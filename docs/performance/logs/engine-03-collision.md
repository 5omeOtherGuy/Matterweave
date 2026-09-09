# Engine-03 collision cadence — engineering log (muse-04 repair)

## Policy chosen: journal-sourced added-solid region gate + staged sync fallback

Whole-collider AABB comparison cannot establish unchanged shape (filling an
interior hole leaves the outer AABB identical), so collider equivalence was
removed rather than repaired. The gate is now sourced from the owner's edit
journal, at cell granularity:

- `DetailCollisionCadence::on_edit(scene, physics, added)` takes the
  world-space `(min, max)` boxes where the edit added solid collision
  material. `Some(&[])` = removal / non-solid edit (never gates).
  `None` = structural change (added/removed/moved instances) and latches a
  conservative mode that defers while any body overlaps any prepared
  collider.
- Regions accumulate across rapid edits, clear on every accepted publication
  and on preparation failure. Bounded by `MAX_PENDING_ADDED` (4096);
  overflow latches conservative mode instead of growing.
- `step` retains (never drops/forces) a gated result and retries later.
- Worker-unavailable fallback builds synchronously and applies the SAME
  gate: clear publishes (`Ok(false)`), blocked stages in one bounded slot
  (`Ok(true)`) for a later `step`. No ungated synchronous path exists.
- Rollback: `PendingEdit` carries the prior save-journal entry
  (`journal: Option<u8>`), restoring confirmed history (e.g. a confirmed
  removal re-added by a failed burst comes back as a removal, not deleted).
- `AsyncDetailCollision::without_worker` added for fallback-path tests.
- Removed `Physics::detail_publication_blocked`; added
  `Physics::dynamic_body_aabbs`, `Physics::detail_added_blocked`, and
  `PreparedDetailCollision::collider_aabbs` (structural gate only).
- Fixed WIP breakage: in-crate `take_result` test missing the `gate`
  argument after the `poll_retaining` change.

## Truthful semantics / latency

- Source/render/query/physics: authoritative scene mutates immediately and
  render leads; live collision = last accepted publication until the gate
  clears. Added solid material is visible-but-not-solid while pending;
  removed walls are solid-but-invisible. No new wall appears through an
  advanced body; no silent stale restore (version tags unchanged).
- Gate overlap is AABB-level (conservative direction: may defer spuriously,
  never publishes through a body).
- Costs: sync fallback = full-scene rebuild on the sim thread + at most one
  staged rebuild per source version; pending region ≤ 4096 × 24 B + one
  staged preparation; gate scan O(added regions × bodies) per frame while a
  current result is buffered (removals cost nothing).
- Collider AABBs reflect the last stepped state (teleport moves the body;
  the collider transform syncs on the next step); gate + publish run
  atomically w.r.t. stepping on the sim thread, so this is sound.

## Verification (all with CARGO_BUILD_JOBS=1, reused debug target)

- `cargo test -p matterweave-physics`: 71 passed, 2 ignored (8 suites).
  Includes 9/9 `detail_cadence` synchronization, incl. 2 new:
  `interior_fill_with_equal_outer_aabb_never_publishes_through_the_body`
  and `unavailable_worker_stages_blocked_fallback_and_publishes_after_clearing`.
- RED checkpoints (verified, then restored): gate neutered
  (`detail_added_blocked → false`) → both new tests FAIL; legacy
  journal-removal logic → `pending_rollback_restores_confirmed_journal_removal`
  FAILS with journal wiped (`left: []`), plus the reinsert arm.
- `cargo test -p matterweave-explorer`: 74 passed, 1 ignored (full-map,
  opt-in). Includes 3 new `pending_rollback_*` journal tests.
- `cargo clippy -p matterweave-physics -p matterweave-explorer --all-targets
  -- -D warnings`: 0 errors in our crates (1 pre-existing warning in
  vendored winit, out of scope). Also fixed a pre-existing
  `assert_eq!(bool, true)` clippy error in the WIP test.
- Test-note: gate tests step physics once after `teleport` because collider
  AABBs sync on step; documented in-test.

## Limitations / lead-owned

- Native/phone acceptance NOT RUN (lead owns).
- Full-map/opt-in explorer test NOT RUN (ignored, campaign gate).
- Runtime `Runtime.edit` computes one cell box per edit; structural async
  edits must pass `None` (currently Runtime only does cell edits).
- Post-`reset` replacements must publish synchronously with pre-validated
  bodies or via `on_edit(.., None)`; documented on `reset`.

## Lead integration review

Two further actual RED regressions at `8c2631c`:

- A workerless rejected structural request cleared the prior addition regions,
  allowing its retained wall preparation to publish through the capsule.
- A teleport before the next physics step left collider cached poses stale, so
  the gate could publish through the character's new position.

The gate now preserves prior region storage until acceptance/rejection is known,
and derives body AABBs from current rigid-body and collider-local transforms.
Eleven cadence tests pass, including both regressions. Scoped physics/explorer
strict Clippy passes (existing vendored-winit warning only). Synchronous acceptance
also clears the owner's pending rollback journal. Removed per-step cloning of the
addition list; corrected two test deadlines that had compared a freshly created
Instant to itself. Full native/Android integration validation remains pending.

The visible edit can precede collision preparation and overlap deferral. There
is **no fixed maximum publication latency** while a body occupies newly added
material; earlier wording claiming a preparation-time bound was incorrect.
