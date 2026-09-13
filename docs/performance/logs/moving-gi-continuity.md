# Continuous moving-cell GI continuity (issue 47) — 2026-09-13

Closes the last open *functional* requirement in production lighting: a body
moving continuously through the wetland proxy scene must not blank indirect
lighting. The previous two slices established that a radius shortcut is wrong
(PR #57) and that exact per-face dependency retention makes the off window short
but not zero (PR #60). This slice changes the unit the budget fills: with the
same gather distance, quadrature, caps and publication contract, the retained
unit is now the *quadrature sample* rather than the face.

Measured on the identical fixture at the stacked base commit `8692e39` and at
this slice's code commit, the production-shaped motion matrix goes from **34 of
48 configurations going dark at least once (minimum live fraction 0.000, 16 of
them dark on every frame)** to **48 of 48 at a live fraction of 1.000, with 177
fresh-reference checkpoints and zero stale slots**. The two-segment counterexample and the coverage-change rule are intact,
and `GATHER_DISTANCE_M = 24.0`, `SAMPLES = 16`, `UPDATE_BUDGET {rays: 1024, work:
8192}` and every cap are unchanged.

**Scope.** `crates/matterweave-render/src/indirect.rs`, its submodule
`crates/matterweave-render/src/indirect/dependency.rs`, the test-only probe
`crates/matterweave-render/src/moving_lighting_probe.rs` (the PR #57 fixture
infrastructure this brief names, extended rather than replaced),
`apps/explorer/src/wetland_lighting.rs` and this log. No shader, no `lib.rs`, no
`world.wgsl`, no budget, gather-distance or sample-count change, no new
publication mode, and no stale or partial lighting is ever uploaded. Host-only:
no device, no phone, no APK, no `/mnt/bench`, no wall-clock, no thermal and no
frame-rate claim. The Android visual functional gate for continuous motion is
**NOT RUN** and belongs to the phone owner.

## Actions Taken

- Read the three settled results (`indirect-dependency-retention.md`,
  `moving-lighting-feasibility.md`, `deepseek-moving-lighting-probe/result.json`)
  and checked each load-bearing claim against the frozen source at `8692e39`:
  the two-segment trace, the `ceil(R)` rejection, the complete-only publication
  gate (`complete`/`valid_for`/`source_valid`), the unmoved
  `GATHER_DISTANCE_M = 24.0` / `SAMPLES = 16` / `UPDATE_BUDGET`, and the per-face
  dependency bitsets of PR #60.
- Built a **continuous-motion fixture** on the PR #57 probe infrastructure
  (`moving_lighting_probe.rs`): the production coverage box (20x10x20 at the
  wetland clearing's own origin `[-10, -5, -10]`), a ground plate plus authored
  props or a pillar field, the three suns the app cycles through
  (`Action::Sun` in `apps/explorer/src/lib.rs`), an aligned and a
  boundary-straddling body footprint, and a bouncing path that stays inside the
  box. One production budget slice per frame, `replace_mesh_proxy` per move,
  `valid_for` as the live test, and fresh-reference bit-exact equality at
  checkpoints.
- Ran that fixture at the frozen base `8692e39` to get the honest baseline
  (34/48 configurations blanked; failing configurations sat pinned at the 1024
  ray cap and never drained), then implemented the fix and re-ran the identical
  fixture.
- Implemented sample-level retention: per-`(face, sample)` dependency sets over
  the coverage box coarsened to 2-cell blocks, per-sample retained
  contributions, a narrowed exposure rule, and a tracker reset whenever the
  volume clears its values. Rewrote the `update` sample loop to replay retained
  samples in ascending order so a recomputed face is bit-identical to a fresh
  one.
- Added a regression test that distinguishes the two mechanisms
  (`a_one_cell_edit_replays_retained_samples_instead_of_whole_faces`: a single
  move spends 306/636 rays against a 1 792/2 080-ray per-face re-trace ceiling),
  a face-order contract test, and a sustained-walk test through the production
  `WetlandLighting` state machine with the `FakeSink` re-checking every
  renderer acceptance condition.
- Ran the render and explorer lib suites, clippy `-D warnings` for both crates,
  `cargo fmt -- --check` and `python3 tools/check_docs.py`.

## Issues & Friction

- **The first fixture was too easy and would have produced a flattering
  answer.** A single-cell body over a ground plate at one cell per frame stayed
  live at 1.000 even *before* the fix, because a body that is one cell wide
  changes only two cells per frame and the ground's facing faces are few. The
  fixture now also models a boundary-straddling footprint (a half-metre physics
  body at a fractional translation, the app's own `pebble` case), which changes
  four cells per step, and a pillar field so most gather and shadow segments end
  on geometry instead of on the sky. Only then does the matrix show the outage
  the requirement is about: at the low sun over the pillar field an aligned body
  was live for 0.100 of frames and a straddling body for 0.000.
- **Motion *rate* is not the same as invalidation *size*.** 8 and 16 cells per
  frame change exactly the same two cells as one cell per frame - the body never
  occupies the cells it skipped - so a faster body is a *shorter* run, not a
  harder one. The rate axis is therefore reported as measured but is not the
  boundary probe; the boundary probe is the wider footprint. This is the sort of
  thing that would have read as "live even at 16 cells/frame" with no caveat.
- **Coarsening the dependency index space was forced by memory, and it costs
  invalidations.** A per-sample dependency set at cell resolution costs
  `16 * 504 = 8 064` bytes plus contributions per face, i.e. ~10 MB for the
  motion fixture's 1 347 sampled faces and more than the engine's 16 MiB cap for
  a denser scene. Block granularity (2 cells per axis, 8 words = 64 bytes per
  sample) brings a face to 1 216 bytes and the whole fixture to 1.6 MiB, and the
  price is visible in the numbers: `invalidated_faces` per move *rises* (for
  example 38 -> 96 in the afternoon clearing fixture) while the rays that must
  be traced *fall* (753 -> 509). The invalidated count is no longer a proxy for
  cost and must not be read as one.
- **Narrowing the exposure rule changed a rule the PR #60 review audited.** The
  reviewed rule invalidated all six faces of each of a changed cell's seven-cell
  closed neighborhood (42 face slots per changed cell). Only twelve of those
  faces have an exposure test that reads the changed cell: the six the cell owns
  and the six its axis neighbours own looking back. The narrower rule is exact
  (every other face's dependence on the changed cell is a traced segment, which
  the sample rule decides), but it is a change to an audited rule, so it is
  called out here and covered by the existing exposure, enclosing and
  fresh-equality tests.
- **`IndirectVolume::clear` had to start resetting the tracker.** A retained
  contribution is only meaningful for the value it was added into. Clearing the
  values without clearing the records would let the next scan replay a
  contribution into a value that no longer exists - a wrong value, and not one
  any existing test would have caught, because the per-face scheme only *read*
  its bitsets for invalidation decisions. The reset is now part of `clear`, and
  a retained-sample replay that survives a full clear is impossible.
- **One existing edge test caught an unrelated memory regression immediately.**
  `indirect_edge_tests::absolute_work_caps_and_lookup_bounds_hold` pins
  `resident_bytes() < 400 000` for a 4 096-cell volume, and the new tracker's
  inline fields pushed `size_of::<IndirectVolume>()` over it by a few bytes. The
  tracker is now boxed, so the volume's inline footprint no longer depends on the
  tracker's field list, and the pinned bound holds unchanged.

## Decisions & Rationale

- **Make the retained unit smaller, not the dependency rule weaker.** The
  brief's starting hypothesis was "a smaller budget-filling unit (fewer faces per
  key)". A moved body cell usually crosses one or two of a face's sixteen
  quadrature rays, so a face-level answer has to re-trace all sixteen to refresh
  those one or two. The sample is the smallest unit whose completeness is
  independently decidable and whose value can be kept, so a sample whose recorded
  dependency set survives an edit is replayed instead of re-traced. Nothing about
  what counts as a dependency is relaxed: the recorded set is still "every proxy
  cell the traced segment reads", and a sample is only replayed when none of them
  changed.
- **Publication stays whole-volume and complete-current.** No publication unit
  was made finer. The volume still publishes only when every face is finished for
  the current key, `complete`/`valid_for`/`source_valid`/the upload gate keep
  their meaning, invalidated slots stay zeroed and superseded keys stay rejected.
  Sample retention only shortens the recompute; it never makes a partially
  recomputed volume visible. The renderer's upload path is whole-buffer and was
  not touched.
- **Coarsen the index space to 2-cell blocks.** Per-sample sets at cell
  resolution cannot fit a bounded cap at the production sample count, and the
  record is worth nothing if faces fall back to `UNTRACKED`. A block-granular
  set is a superset of the read set, so it invalidates *more* samples than exact
  cell tracking and can never retain a stale one. `MAX_DEPENDENCY_BYTES`
  (16 MiB engine, 6 MiB app) is unchanged.
- **Keep the per-sample contributions and replay them in order.** The value is
  `sum(contributions) / samples` accumulated in ascending sample order, so a
  partially retained face must re-add its survivors in that order rather than
  subtract or re-sum. Storing the exact `f32` vector that was added makes the
  replay bit-identical, which is what the fresh-reference checkpoints verify.
  12 bytes per sample buys that.
- **Reset the tracker whenever the volume clears its values.** See "Issues &
  Friction": without it a cleared volume could replay a stale contribution. This
  is the same class of hazard as review finding F1 on PR #60 - a record is only
  valid inside the frame of reference it was made in, and here that frame is
  "the value this contribution was added into".
- **Leave every production constant alone.** `GATHER_DISTANCE_M = 24.0`,
  `SAMPLES = 16`, `UPDATE_BUDGET {rays: 1024, work: 8192}`,
  `INDIRECT_DEPENDENCY_BYTES = 6 MiB` and `MAX_DEPENDENCY_BYTES = 16 MiB` are
  unchanged, so no fresh-reference equality test at new values was needed. The
  measured worst frame uses 1 023 of the 1 024 budget rays (see "Insights" for
  how narrow that is).
- **Report the baseline against the same fixture, from the base commit.** The
  baseline in this log was produced by checking out `8692e39`'s two production
  files into this worktree and running the identical test, not by quoting the
  earlier slice's different fixture. The comparison is therefore
  fixture-controlled.

## Solutions Applied

### Source contract

- **Recorded per sample** (`dependency.rs`): sample `i` of a face records the
  block of the face's own cell (the exposure test reads it and every segment
  starts there), the block of the outward neighbour for `i = 0`, and the blocks
  of every proxy cell the gather segment and the hit-to-sun segment read. Each
  segment is clipped with `MeshProxy::segment_exit(..., limit)` where `limit` is
  what `trace_scene_limit` used, so a segment that never enters the coverage box
  records nothing. Recorded cells are mapped through `block_of` (2 cells per
  axis); a traced sample always records at least its origin cell, so an empty
  bitset means "not recorded" and is re-traced.
- **Retained contributions**: `store_contribution` writes exactly the `Vec3`
  that the shipped loop adds to `sum`; `retained_contribution` returns it only
  when the sample's bitset is non-empty. `reset_face` (an enclosed face) and
  `invalidate_edit` (a crossed sample) zero both the bitset and the
  contribution, and `reset` clears the whole arena.
- **Invalidation on `replace_mesh_proxy`** stays a single ascending two-pointer
  diff over `(cell, material)`, then:
  - the *exposure* rule clears exactly the faces whose exposure test reads a
    changed cell (`cell`'s six faces plus the six its axis neighbours own
    looking back);
  - the *sample* rule drops every sample whose recorded blocks intersect the
    changed blocks, which clears its face's completed bit and zeroes its value;
    an `UNTRACKED` face is always dropped.
  Every changed cell outside the index space, a `None` <-> `Some` transition and
  any change to the proxy's `origin`/`dimensions` are still coverage changes
  (`tracked = false`) that clear every value - PR #60's finding F1 rule,
  unchanged and still tested.
- **Replay in the update loop** (`indirect.rs`): a face visit walks samples
  `0..samples` in ascending order. A retained sample adds its stored
  contribution and costs one work unit and no ray; a non-retained sample traces
  as before. The face is written once, at the end of the loop, exactly as
  before, so the completion marker and `done` bit keep their meaning. The ray
  cap is checked only when a ray is actually needed, so a frame with no rays
  available can still finish faces whose samples are all retained.
- **Tracker ownership**: `IndirectVolume::clear` calls `tracking.reset()`;
  `enable_proxy_retention` takes `samples` so the arena is sized per sample; the
  tracker is boxed so its fields do not grow the volume's inline footprint.
- **Public-surface delta**: `RetentionStatus::bits_per_face` is renamed to
  `dependency_cells` (the index space is now blocks, so the old name would have
  been wrong by 8x) and `block_axis` / `samples` are added. `resident_bytes`,
  `cap_bytes`, `tracked_faces`, `untracked_faces`, `face_slots` and
  `outside_cells` keep their names and meanings. The app consumes only
  `resident_bytes`.
- **API compatibility**: `set_mesh_proxy`, `new`, `update`, `complete`,
  `valid_for`, `source_valid`, `sample`, `mesh_digest`, `has_mesh_proxy`,
  `pending_work`, `dirty_faces` and `resident_bytes` keep their contracts. With
  retention disabled the volume takes the same code path it always did.

### Continuous-motion fixture (`moving_lighting_probe.rs`)

- Coverage box `[-10, -5, -10]` for 20x10x20 cells, the production origin the
  wetland clearing rounds to; ground plate at cell level `y = 0`; the body path
  bounces along `z = -6` between two cells so every frame moves and stays
  inside the box.
- Two scene shapes: `Clearing` (ground plus nine authored props) and `Debris`
  (ground plus a two-cell pillar field every third cell, ~100 extra cells, the
  densest cover the fixture models).
- Two footprints: `Aligned` (one cell; a step changes two cells) and `Straddle`
  (2x2; a step changes four cells, the largest footprint a body resting on the
  ground can have).
- Three production suns at intensity 0.8: `afternoon [0.4, 0.85, 0.3]`,
  `low [-0.8, 0.35, 0.3]`, `overhead [0.2, 1.0, -0.5]`.
- Rates 1/2/4/8/16 cells per frame for `Aligned` and 1/2/4 for `Straddle`;
  30 body cells per run; one production budget slice per frame.
- Every live checkpoint recomputes the volume from scratch for that frame's own
  proxy and compares all 24 000 slots bit-for-bit. The dense healing
  configuration prints its per-frame counters.

### Baseline vs this slice (identical fixture, identical test)

Base `8692e39` (per-face retention) -> this slice (per-sample retention). Full
matrix; `live` is the fraction of frames whose volume was complete and current
for the frame's own proxy, and `rays` is the largest single-frame ray spend (the
budget is 1 024; a capped value means the frame did not drain).

| config | live before | live after | rays before | rays after | max invalidated before | max invalidated after |
| --- | --- | --- | --- | --- | --- | --- |
| afternoon/Clearing/Aligned/1cells | 1.000 | 1.000 | 753 | 509 | 38 | 96 |
| afternoon/Clearing/Aligned/2cells | 1.000 | 1.000 | 817 | 509 | 42 | 96 |
| afternoon/Clearing/Aligned/4cells | 1.000 | 1.000 | 919 | 530 | 48 | 115 |
| afternoon/Clearing/Aligned/8cells | 1.000 | 1.000 | 949 | 525 | 51 | 126 |
| afternoon/Clearing/Aligned/16cells | 1.000 | 1.000 | 842 | 441 | 47 | 126 |
| afternoon/Clearing/Straddle/1cells | 0.333 | 1.000 | 1024 | 580 | 58 | 98 |
| afternoon/Clearing/Straddle/2cells | 0.067 | 1.000 | 1024 | 580 | 67 | 98 |
| afternoon/Clearing/Straddle/4cells | 0.125 | 1.000 | 1024 | 635 | 73 | 119 |
| afternoon/Debris/Aligned/1cells | 0.733 | 1.000 | 1024 | 689 | 48 | 108 |
| afternoon/Debris/Aligned/2cells | 0.067 | 1.000 | 1024 | 693 | 54 | 108 |
| afternoon/Debris/Aligned/4cells | 0.125 | 1.000 | 1024 | 797 | 61 | 123 |
| afternoon/Debris/Aligned/8cells | 0.000 | 1.000 | 1024 | 804 | 64 | 142 |
| afternoon/Debris/Aligned/16cells | 0.000 | 1.000 | 1024 | 795 | 63 | 142 |
| afternoon/Debris/Straddle/1cells | 0.000 | 1.000 | 1024 | 781 | 68 | 112 |
| afternoon/Debris/Straddle/2cells | 0.000 | 1.000 | 1024 | 781 | 83 | 112 |
| afternoon/Debris/Straddle/4cells | 0.000 | 1.000 | 1024 | 885 | 98 | 125 |
| low/Clearing/Aligned/1cells | 1.000 | 1.000 | 800 | 544 | 41 | 104 |
| low/Clearing/Aligned/2cells | 1.000 | 1.000 | 888 | 544 | 46 | 104 |
| low/Clearing/Aligned/4cells | 1.000 | 1.000 | 955 | 602 | 50 | 124 |
| low/Clearing/Aligned/8cells | 0.750 | 1.000 | 1023 | 580 | 55 | 133 |
| low/Clearing/Aligned/16cells | 1.000 | 1.000 | 823 | 498 | 46 | 136 |
| low/Clearing/Straddle/1cells | 0.267 | 1.000 | 1024 | 640 | 60 | 106 |
| low/Clearing/Straddle/2cells | 0.067 | 1.000 | 1024 | 640 | 70 | 106 |
| low/Clearing/Straddle/4cells | 0.125 | 1.000 | 1023 | 683 | 77 | 128 |
| low/Debris/Aligned/1cells | 0.100 | 1.000 | 1024 | 796 | 51 | 130 |
| low/Debris/Aligned/2cells | 0.067 | 1.000 | 1024 | 796 | 57 | 130 |
| low/Debris/Aligned/4cells | 0.000 | 1.000 | 1024 | 931 | 69 | 148 |
| low/Debris/Aligned/8cells | 0.000 | 1.000 | 1023 | 981 | 77 | 178 |
| low/Debris/Aligned/16cells | 0.000 | 1.000 | 1023 | 917 | 75 | 172 |
| low/Debris/Straddle/1cells | 0.000 | 1.000 | 1024 | 888 | 73 | 134 |
| low/Debris/Straddle/2cells | 0.000 | 1.000 | 1024 | 874 | 90 | 134 |
| low/Debris/Straddle/4cells | 0.000 | 1.000 | 1024 | 1023 | 105 | 149 |
| overhead/Clearing/Aligned/1cells | 1.000 | 1.000 | 776 | 516 | 39 | 98 |
| overhead/Clearing/Aligned/2cells | 1.000 | 1.000 | 817 | 516 | 42 | 98 |
| overhead/Clearing/Aligned/4cells | 1.000 | 1.000 | 921 | 526 | 48 | 116 |
| overhead/Clearing/Aligned/8cells | 1.000 | 1.000 | 929 | 536 | 50 | 127 |
| overhead/Clearing/Aligned/16cells | 1.000 | 1.000 | 822 | 451 | 46 | 127 |
| overhead/Clearing/Straddle/1cells | 0.333 | 1.000 | 1024 | 583 | 58 | 100 |
| overhead/Clearing/Straddle/2cells | 0.067 | 1.000 | 1024 | 583 | 67 | 100 |
| overhead/Clearing/Straddle/4cells | 0.125 | 1.000 | 1024 | 634 | 73 | 120 |
| overhead/Debris/Aligned/1cells | 0.500 | 1.000 | 1024 | 710 | 47 | 112 |
| overhead/Debris/Aligned/2cells | 0.067 | 1.000 | 1024 | 709 | 54 | 112 |
| overhead/Debris/Aligned/4cells | 0.125 | 1.000 | 1023 | 804 | 61 | 126 |
| overhead/Debris/Aligned/8cells | 0.000 | 1.000 | 1023 | 799 | 65 | 145 |
| overhead/Debris/Aligned/16cells | 0.000 | 1.000 | 1023 | 799 | 64 | 145 |
| overhead/Debris/Straddle/1cells | 0.000 | 1.000 | 1024 | 803 | 66 | 116 |
| overhead/Debris/Straddle/2cells | 0.000 | 1.000 | 1024 | 802 | 81 | 116 |
| overhead/Debris/Straddle/4cells | 0.000 | 1.000 | 1024 | 897 | 96 | 128 |

Summary of the same run: **before** 34 of 48 configurations below 1.000, minimum
0.000, every failing configuration sitting at the ray cap (1 023 or 1 024 rays)
and never draining; **after** 0 of 48 below 1.000, minimum 1.000, 177 checkpoints compared
against fresh references with 0 differing slots, 0 faces left `UNTRACKED`, and a
worst single-frame spend of 1 023 rays (`low/Debris/Straddle/4cells`) and 2 866
work units (`low/Debris/Aligned/8cells`, budget 8 192).

The baseline is not the "one dark frame per move" the retention slice predicted:
once the invalidated set exceeds one slice, the next move invalidates more before
the volume finishes, so the dirty set never drains and the volume stays dark for
the rest of the walk. That is why the baseline column is dominated by 0.000
rather than by short dips.

### Per-frame counters, dense healing configuration

`low/Debris/Aligned/1cells` — the low sun over the pillar field, an aligned body
at one cell per frame, 30 frames. `retained`/`invalidated` are that frame's edit
counts, `dirty`/`pending` are read after the frame's budget slice, `rays`/`work`
are that frame's spend.

```
step=1  live=true retained=1186 invalidated=90  dirty=0 pending=0 rays=485 work=1425
step=2  live=true retained=1154 invalidated=120 dirty=0 pending=0 rays=765 work=1967
step=3  live=true retained=1176 invalidated=100 dirty=0 pending=0 rays=547 work=1616
step=4  live=true retained=1154 invalidated=122 dirty=0 pending=0 rays=735 work=1937
step=5  live=true retained=1183 invalidated=91  dirty=0 pending=0 rays=546 work=1503
step=6  live=true retained=1151 invalidated=125 dirty=0 pending=0 rays=796 work=2016
step=7  live=true retained=1178 invalidated=98  dirty=0 pending=0 rays=499 work=1553
step=8  live=true retained=1146 invalidated=128 dirty=0 pending=0 rays=779 work=2095
step=9  live=true retained=1175 invalidated=101 dirty=0 pending=0 rays=549 work=1632
step=10 live=true retained=1154 invalidated=122 dirty=0 pending=0 rays=735 work=1937
step=11 live=true retained=1185 invalidated=89  dirty=0 pending=0 rays=542 work=1471
step=12 live=true retained=1161 invalidated=115 dirty=0 pending=0 rays=768 work=1856
step=13 live=true retained=1188 invalidated=88  dirty=0 pending=0 rays=472 work=1393
step=14 live=true retained=1171 invalidated=103 dirty=0 pending=0 rays=700 work=1695
step=15 live=true retained=1200 invalidated=76  dirty=0 pending=0 rays=472 work=1232
step=16 live=true retained=1200 invalidated=76  dirty=0 pending=0 rays=470 work=1232
step=17 live=true retained=1171 invalidated=105 dirty=0 pending=0 rays=653 work=1665
step=18 live=true retained=1188 invalidated=86  dirty=0 pending=0 rays=518 work=1423
step=19 live=true retained=1161 invalidated=115 dirty=0 pending=0 rays=767 work=1856
step=20 live=true retained=1185 invalidated=91  dirty=0 pending=0 rays=495 work=1441
step=21 live=true retained=1154 invalidated=120 dirty=0 pending=0 rays=782 work=1967
step=22 live=true retained=1175 invalidated=101 dirty=0 pending=0 rays=549 work=1632
step=23 live=true retained=1146 invalidated=130 dirty=0 pending=0 rays=732 work=2065
step=24 live=true retained=1178 invalidated=96  dirty=0 pending=0 rays=546 work=1583
step=25 live=true retained=1151 invalidated=125 dirty=0 pending=0 rays=796 work=2016
step=26 live=true retained=1183 invalidated=93  dirty=0 pending=0 rays=499 work=1473
step=27 live=true retained=1154 invalidated=120 dirty=0 pending=0 rays=782 work=1967
step=28 live=true retained=1176 invalidated=100 dirty=0 pending=0 rays=547 work=1616
step=29 live=true retained=1154 invalidated=122 dirty=0 pending=0 rays=718 work=1937
step=30 live=true retained=1186 invalidated=88  dirty=0 pending=0 rays=532 work=1455
```

### Landing, cap and app evidence

- **Per-sample replay (mechanism regression test, `sparse`/`dense` of the PR #60
  fixtures, PR #57's oblique sun, one production slice per iteration):**

```
[retention] sparse sample replay: invalidated=112 frames=1 rays=306 work=1808 face_retrace_ceiling=1792
[retention] dense  sample replay: invalidated=130 frames=1 rays=636 work=2127 face_retrace_ceiling=2080
```

  `face_retrace_ceiling = invalidated_faces * SAMPLES` is a floor for per-face
  retention, because every invalidated face traces at least one ray per sample.
  Spending 306 and 636 rays against 1 792 and 2 080 is what distinguishes a
  sample-level answer from a face-level one, and the same test re-checks the
  replayed volume bit-for-bit against a fresh reference.
- **Wetland app fixture** (`a_body_cell_move_retains_the_untouched_gi_faces`, the
  PR #60 fixture): before `retained=39 invalidated=19 dependency_kib=124`, after
  `retained=32 invalidated=26 dependency_kib=168`, `dirty=0 pending=0
  gi_live=true`. The invalidated count rises because the block-granular sample
  rule is conservative and because the exposure rule now clears exactly the
  faces it must; the frame still drains.
- **Sustained walk through the production scheduler**
  (`a_sustained_body_walk_republishes_gi_on_every_frame`): five consecutive
  one-cell frames through `WetlandLighting`, the renderer's install disabling GI
  before each one, the `FakeSink` re-checking `complete()`, `valid_for`, the
  digest match and the retired-digest rule on every upload:

```
[wetland walk] steps=5 live=true invalidated=[(0, 23, 39), (1, 41, 21), (2, 28, 32), (3, 38, 24), (4, 24, 40)]
```

  (tuples are `(step, invalidated_faces, retained_faces)`), with
  `sink.violations.is_empty()` asserted.
- **Dependency rule intact**: PR #57's two-segment counterexample
  (`far_occluder_second_segment_is_invalidated_exactly`) still shows the receiver
  five to six cells from the moved body changing `0.06618919 -> 0.05515766` via
  its sun segment, still zeroes it mid-edit, and still converges bit-exactly to
  the fresh reference; the reversed-sun control is still bit-identical. All five
  coverage-change tests (`None` <-> `Some`, box growth, out-of-index cell,
  out-of-box traversal, covered geometry clipped) pass unchanged.
- **Caps and degradation**: `tracking_cap_degrades_to_untracked_never_stale`
  still shows untracked faces always invalidated and never retained stale, and
  `resident_bytes() <= cap_bytes()` exactly; the motion fixture never falls back
  to `UNTRACKED` (0 untracked faces in all 48 configurations).

### Memory accounting (structural, not a benchmark)

| Array | Motion fixture (`Debris`, 20x10x20 = 4 000 cells) | Notes |
| --- | --- | --- |
| offsets | 24 000 × 4 B = 96 KiB | one arena face index per slot |
| cell changed mask (scratch) | 63 words = 504 B | the exposure rule's index space |
| block changed mask (scratch) | 8 words = 64 B | the sample rule's index space |
| per-face records | 16 × (64 B bitsets + 12 B contribution) = 1 216 B | lazily allocated per sampled face |
| measured, aligned run | 1 347 faces tracked, 1 694 KiB resident | `RetentionStatus::resident_bytes` |
| measured, straddle run | 1 392 faces tracked, 1 747 KiB resident | app cap 6 MiB, engine clamp 16 MiB |

A face that does not fit the cap is `UNTRACKED` and recomputed on every edit, so
the cap degrades retention and never correctness.

### Verification (this source)

All Cargo commands used `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1
CARGO_TARGET_DIR=../moving-gi-target` (the exact target directory the brief
names).

- `cargo test --locked -p matterweave-render --lib` -> `127 passed; 0 failed`
  (PR #60's 124 plus the two new dependency tests and the probe's motion test).
- `cargo test --locked -p matterweave-explorer --lib` -> `231 passed; 0 failed;
  1 ignored` (PR #60's 230 plus the sustained-walk test).
- `cargo clippy --locked -p matterweave-render --all-targets -- -D warnings` ->
  exit 0. `cargo clippy --locked -p matterweave-explorer --all-targets -- -D
  warnings` -> exit 0.
- `cargo fmt -p matterweave-render -p matterweave-explorer -- --check` -> clean.
- `python3 tools/check_docs.py` -> `PASS: 238 Markdown files, 676 local links,
  16 ADRs and 20 requirements` (includes this log).
- Every commit reference in this file was checked with `git cat-file -e`:
  `8692e39` (the base), `495fd1b` (the slice), `4b2fdca` and `e6bca89` (the head)
  and the full `e6bca8934523d19be6612d261858dc4fe8c6fc94` all resolve. The only
  other hex-shaped tokens in this file are the decimal fractions
  `0.06618919`/`0.05515766` inside the quoted counterexample values, which are
  numbers and not commits.
- Baseline comparison: `8692e39`'s `indirect.rs` and `indirect/dependency.rs`
  checked out over this worktree, the same motion test run, then the worktree
  restored to this commit. The baseline run fails its own assertions (that is
  the comparison) and printed the "before" column above.

### Behavior delta vs the frozen base (recorded, not hidden)

- **A moving body no longer blanks GI at the measured motion rates.** The
  production scheduler can publish a complete, current volume in the same frame
  as the footprint change instead of staying dark until the motion pauses.
- **`invalidated_faces` is no longer a cost proxy.** It counts faces whose
  completed bit was cleared, including faces that replay every sample without
  tracing a ray. The cost numbers are `rays` and `work`.
- **More faces are walked per edit** (up to 34% more work per frame in the
  measured matrix: worst 2 866 of 8 192 work units, against a 1 298 worst
  baseline). The trade is bounded and the ray budget is the binding constraint
  in every failing case.
- **`RetentionStatus::bits_per_face` is renamed** to `dependency_cells`, and
  `block_axis`/`samples` are added. The tracker is boxed inside
  `IndirectVolume`.
- **The exposure rule is narrowed** from the changed cell's seven-cell closed
  neighbourhood (42 face slots) to the twelve face slots whose exposure test
  actually reads the changed cell.
- **The tracker resets on `clear`**, which the per-face scheme did not need.

### Delivery snapshot

- Base (PR #60 head, reviewed and fixed): `8692e39`, branch
  `engine/moving-gi-continuity`, stacked directly on it at the brief's
  instruction.
- Commits: `495fd1b` (the slice: `dependency.rs`, `indirect.rs`, the probe
  fixture and the app test), `4b2fdca` (the dense per-frame table) and `e6bca89`
  (the face-order contract test). Nothing amended, rebased or force-pushed.
- Files: `crates/matterweave-render/src/indirect/dependency.rs`,
  `crates/matterweave-render/src/indirect.rs`,
  `crates/matterweave-render/src/moving_lighting_probe.rs`,
  `apps/explorer/src/wetland_lighting.rs`, this log.
- Frozen source SHA for review: `e6bca8934523d19be6612d261858dc4fe8c6fc94`
  (`test(render): pin the dependency face order against the volume slot order`).
  The branch head carries this log only, on top of that commit, so reviewing
  `e6bca89` reviews all of the source and this file documents it; the slice
  commit alone is `495fd1b`.
- Branch pushed as `engine/moving-gi-continuity`; **not** merged, not
  self-approved and no PR opened, per the brief.

### Proposed shared updates (log only, not applied here)

- `docs/STATUS.md`: replace "continuous moving-body GI continuity remains an
  open requirement" with "sample-level dependency retention keeps indirect
  lighting live at every measured motion rate and footprint in the host
  fixture; the Android visual gate for continuous motion is still NOT RUN".
- `docs/performance/README.md`: add an "Engine techniques" entry linking
  `logs/moving-gi-continuity.md`.
- Roadmap/board: the moving-lighting item's host half is complete; what remains
  is the phone-owner visual functional gate during continuous motion, which no
  host counter can substitute for.

## Insights

- **The unit of publication was never the problem; the unit of recompute was.**
  The requirement is only hard because the invalidated set must finish inside one
  budget slice, and the budget is spent on *rays*. A face-level answer spends
  sixteen rays to refresh one crossed ray; a sample-level answer spends one.
  Making the retained unit smaller is what bought continuity, and it did so
  without touching the publication contract, the quadrature, the gather distance
  or the budget.
- **Failure is a cliff, not a dip.** When a frame's invalidated set does not fit
  the ray budget, the next move invalidates more before the volume finishes, so
  the set never drains and GI stays dark for the whole walk. In the baseline
  matrix 34 of 48 configurations went dark and 16 of those were dark for *every*
  frame. That is why a partial improvement (a shorter off window) could never
  have satisfied this requirement, and why the earlier slice's own conclusion
  ("partial publication alone does not achieve usable motion") pointed at the
  recompute unit rather than at a publication mode.
- **The honest boundary is the ray budget, and it is closer than it looks.**
  Every measured configuration drains inside one frame, but the hardest one -
  a boundary-straddling footprint at 4 cells per frame under the low sun in the
  pillar field, i.e. 8 changed cells per frame - uses 1 023 of the 1 024 budget
  rays. At the rate the requirement actually names (one cell per frame) the worst
  configuration uses 888 of 1 024, and an *aligned* body at that rate uses 509 to
  796. The mechanism is what changed; the budget is unchanged and is now the
  binding limit, which is the number a future slice should move if the real
  wetland scene turns out to be denser than this fixture.
- **Coarser dependency records trade invalidations for memory, and the trade is
  visible.** Block-granular sets invalidate more faces (38 -> 96 per move in the
  clearing fixture) and less than a third of the rays are needed. Anyone reading
  the retention counters should read `rays` and `work`, not `invalidated`.
- **A record is only valid inside its frame of reference, and this slice had its
  own instance of it.** PR #60's finding F1 was "a bitset is the read set only of
  the box it was recorded under"; here it is "a retained contribution is only
  meaningful for the value it was added into". Clearing values without clearing
  records would have replayed a stale contribution into a fresh value, and only
  the `clear`-resets-tracker rule prevents it. The general lesson repeats: for
  every retained datum, name the state it is coherent with, and reset it exactly
  when that state is discarded.
- **A fixture can hide the failure it is meant to expose.** The first version of
  this fixture stayed live before the fix. The two things that made it honest
  were a body footprint that straddles cell boundaries (real physics bodies do)
  and a scene where most rays end on geometry rather than on the sky. Both are
  properties of the production wetland, not artificial hardness.
- **The Android visual functional gate is still the only evidence that closes
  this requirement as *seen*.** Everything here is host counters in engine work
  units on a fixture. A phone owner should run a scripted body path with the
  per-frame `retained`/`invalidated`/`dirty`/`pending`/`gi` report line and
  compare the visible result against a per-frame reference; if the real scene is
  denser than this fixture, the number to move is `UPDATE_BUDGET.rays`, not the
  dependency rule.

| Criterion | Verification method | Owner | Result |
| --- | --- | --- | --- |
| Invariant preserved | No stale/partial publication reachable; fresh-reference equality at every checkpoint | this worker | **PASS (host)** — 177 checkpoints over 48 configurations compared all 24 000 slots bit-for-bit against a fresh recompute for the same proxy, 0 differing slots; the volume still publishes only when `complete()`/`valid_for`/`source_valid` hold, invalidated slots stay zeroed, superseded keys stay rejected, and the app walk keeps the `FakeSink`'s acceptance checks green |
| Continuity characterized | Per-frame live/dirty counts over a sustained path at >=2 motion rates, honest fractions | this worker | **PASS (host)** — 48 configurations (2 scenes x 2 footprints x 3 production suns x 1/2/4/8/16 cells per frame), 30 body cells each, one production slice per frame: 48/48 live at 1.000 (base `8692e39`: 34/48 blanking, minimum 0.000). Limits recorded: worst frame 1 023/1 024 rays at the extreme configuration, 888/1 024 at one cell per frame with the straddling footprint, 544-796 at one cell per frame aligned; worst work 2 866/8 192 |
| Dependency rule intact | Probe-57 counterexample still caught; coverage-change full clear still correct | this worker | **PASS (host)** — `far_occluder_second_segment_is_invalidated_exactly` (sun segment to a receiver 5-6 cells away, reversed-sun control bit-identical), all five coverage-change tests, the exposure/enclosing/pending-edit/key tests, and the new sample-replay regression test (306/636 rays against a 1 792/2 080 face-retrace ceiling) |
| Constants justified | R 24 / SAMPLES 16 unchanged; any budget change justified and re-tested | this worker | **PASS** — `GATHER_DISTANCE_M = 24.0`, `SAMPLES = 16`, `UPDATE_BUDGET {rays: 1024, work: 8192}`, `INDIRECT_DEPENDENCY_BYTES = 6 MiB`, `MAX_DEPENDENCY_BYTES = 16 MiB` are all unchanged (diff-limited), so no re-test at new values was required; every test uses the production values |
| Checks green | render + explorer lib tests, clippy -D warnings, fmt, tools/check_docs.py | this worker | **PASS** — 127 + 231 tests, clippy `-D warnings` exit 0 for both crates (all targets for render, all targets for explorer), `cargo fmt -- --check` clean, `tools/check_docs.py` PASS (238 Markdown files, 676 local links, 16 ADRs, 20 requirements) |
| Log and frozen SHA | Five headings filled, honest limits recorded, branch pushed, SHA reported | this worker | **PASS** — this file; frozen source `e6bca8934523d19be6612d261858dc4fe8c6fc94` with this log as its only child commit, slice commit `495fd1b`; branch `engine/moving-gi-continuity` pushed |
| Independent review | Corrective review of this slice | Orchestrator dispatch | **NOT RUN** |
| Android visual gate | Phone-owner functional gate during continuous motion | Owner | **NOT RUN** — no device, APK, phone, wall-clock, thermal or frame-rate claim is made anywhere in this log |
