# Continuous moving-body GI dependency probe (issue 47) — 2026-09-13

Executable functional feasibility probe for the dependency-bounded invalidation
proposed for continuous moving-body GI. It answers the review that rejected the
proposal (`../orchestration/opus-moving-lighting-feasibility/result.json`,
against `../orchestration/opus-moving-lighting-slice/result.json`) with measured
counts instead of arithmetic: the proposal's `ceil(R)` dirty radius is wrong,
the conservative radius is `ceil(2R) + 1`, and at that radius the dirty set is
still too large and too slow to make GI live during motion.

**Scope.** New test-only module
`crates/matterweave-render/src/moving_lighting_probe.rs` plus the two-line
`#[cfg(test)]` registration in `crates/matterweave-render/src/lib.rs` and this
log. No production algorithm, budget, gather distance, shader, app source or
`docs/STATUS.md` change; **no `R` is adopted**. The performance log index
(`docs/performance/README.md`) is not updated either: the owned writes for this
probe are the module, its test registration and this log. Host-only; no device,
no phone, no `/mnt/bench`, no SSD benchmark. Ray and work numbers are engine work
units derived from the published wetland budget, **not** time, and no latency
claim is made from them. No full-GI-completion claim.

Everything below comes from the frozen source `engine/wetland-proxy-lighting`
head (`3d33c4f`, the PR54 head this probe branches from) and was executed from
it before the delivery commit.

## Actions Taken

- Read both orchestration results and checked their load-bearing claims against
  the frozen `crates/matterweave-render/src/indirect.rs`: one sample traces two
  segments capped at `self.distance` (gather, then hit-to-sun from the hit
  point); `IndirectVolume::set_mesh_proxy` unconditionally `clear()`s; the
  wetland shape and budget live in
  `apps/explorer/src/wetland_lighting.rs` (`BOX_DIMENSIONS` 20x10x20 = 4000
  cells, `SAMPLES = 16`, `UPDATE_BUDGET {rays: 1024, work: 8192}`,
  `GATHER_DISTANCE_M = 24.0`).
- Added `crates/matterweave-render/src/moving_lighting_probe.rs` (818 lines,
  `#[cfg(test)]` only) with three evidence paths, all through the shipped code
  (`MeshProxy::build`, `IndirectVolume::new/set_mesh_proxy/update/sample`):
  1. the geometry/work table for `R` in `{1, 2, 4, 8, 24}` on empty, sparse and
     dense surroundings, at an edge and a mid-box body position;
  2. the sun-occluder counterexample at `R = 4` with a reversed-sun control;
  3. fresh before/after volumes per scene and radius, diffed face by face and
     checked against the same-radius bound.
- Derived the conservative cell radius with its offsets, and measured the
  proposal's `ceil(R)` radius against real value changes.
- Ran the probe, the full render lib suite, scoped clippy, `rustfmt` and
  `check_docs.py`; recorded the commands and results below.

## Issues & Friction

- **The first dense fixture produced nothing to compare.** Continuous two-cell
  walls every other `z` made every hit-to-sun segment land on the next wall
  under the oblique probe sun, so every gathered value was zero and the
  "actual differences" check was vacuous (0 changed faces for `R >= 2`).
  Replaced with a checkerboard pillar field that leaves the floor lit through
  the gaps; changed faces became 12-24 (edge) and 12-51 (mid-box).
- **The first bound model overstated the ray cost of a dirty set.** Counting
  the union of the before/after exposure sets as "faces needing rays" is wrong:
  a face covered by the move is zeroed without tracing. The table now separates
  `touched` (union: slots that must be rewritten) from `rays_for` (after
  placement: slots that trace rays).
- **The measured work sits slightly above the slot model.** The shipped loop
  spends one budget iteration when the ray cap interrupts an exposed face
  mid-sample (work counted, no sample progress). Sparse whole-volume work is
  37634 measured against 37620 for the model. The work column is a loop-model
  ceiling; the measured column is the real counter.
- **The `R`-only miss is position- and radius-dependent and mostly not visible
  in generic scenes.** Outside-the-`R`-bound changed faces: sparse 0 at every
  `R`; dense mid-box 6 at `R = 2` and 8 at `R = 4`; the engineered
  counterexample 2 at `R = 4`; every other combination 0. Nothing at the edge
  position in a generic scene exercises it. This is reported as measured, not
  generalised; the counterexample is what makes the bound's insufficiency
  falsifiable.
- **`MeshProxy` is not `Clone` and the volume's proxy copy is private**, so the
  probe rebuilds the identical one-cell proxy for each completed volume and for
  the exposure checks. Trivial cost, noted so a reader does not look for state
  access that does not exist.

## Decisions & Rationale

- **Test-only module, no production edits.** The brief allows a minimal
  test-only module when the public surface cannot express the check, and the
  review's scope forbids touching the shaders, budgets and gather distance.
- **Actual `update`/`sample`, not a copied ray predicate.** The counterexample's
  claim is a pair of completed volumes whose published values differ; the
  revoked-bound claim is the cell distance of that difference. The reversed-sun
  control is an empirical check that the difference is the second segment.
- **Metric = cells, slots, touched/ray faces and budget slices.** No wall-clock.
  Rays and work are engine work units; the brief forbids deriving latency from
  rays alone, and none is derived.
- **Conservative radius `ceil(2R) + 1` cells, derived.** Gather <= `R`, then
  sun visibility <= `R` from a hit point that is itself <= `R` away, so reach is
  `2R`; the sample origin sits `0.5 + eps` off the cell centre and the second
  segment starts `eps` off the hit face, so one cell of index slack covers both.
  The proposal's `ceil(R)` is measured only to falsify it.
- **Two body positions, five radii.** The bound's size is strongly
  position-dependent (see Insights), and the review's "entire volume" line came
  from a bounding-box argument; both the rejected `R = 4` and the production
  `R = 24` are included.
- **Three scenes.** Empty (no geometry: the volume has no radiance to change),
  sparse (floor plus six cells) and dense (floor plus a checkerboard of two-cell
  pillars), so the empty/sparse/dense spread the brief asks for is covered
  rather than asserted.
- **No universal frame claim.** The empty scene's conservative bound at `R = 4`
  drains inside one work slice, so "frames > 1 at every `R`" is false there; the
  probe reports it instead of asserting it.

## Solutions Applied

Changed files: `crates/matterweave-render/src/moving_lighting_probe.rs` (new,
test-only) and `crates/matterweave-render/src/lib.rs` (`#[cfg(test)] mod
moving_lighting_probe;`, two lines), plus this log.

Every Cargo command used
`RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=../wetland-lighting-target`.
The target directory was idle before the runs.

### Fixtures and the derived bound

- Volume: origin `[-10, 0, -10]`, 20 x 10 x 20 = 4000 cells, 24 000 face slots
  (wetland shape and residency cap). `SAMPLES = 16`, `UPDATE_BUDGET` exactly as
  the wetland sets it.
- Body: one unit-cell mesh proxy moving one cell (`edge`:
  `[-9,1,-9] -> [-8,1,-9]`; `center`: `[1,1,1] -> [2,1,1]`). Every scene keeps
  both cells air, so the changed-cell count is exactly 2 (2 occupancy, 0
  material-only) in all 30 scene/position/radius combinations.
- Sun: oblique `[0, 1, 5]` (normalised inside the volume) so the second segment
  has horizontal reach; the wetland's vertical sun is a special case the same
  bound covers.
- Bound cells: in-box cells within `ceil(R)` (rejected radius) or
  `ceil(2R) + 1` (conservative) Chebyshev cells of either changed cell.
- `touched` = slots exposed in either placement; `rays_for` = slots exposed in
  the corrected placement; `rays_exposed = rays_for * SAMPLES * 2`;
  `work_exposed = rays_for * SAMPLES + (slots - rays_for)`; frames are those
  ceils over the wetland budget. `rays_all = slots * SAMPLES * 2` is kept only
  as the review's all-slot ceiling.

### Bound and work table (probe output, abridged)

Rendering of the probe's `[probe]` lines; `fR`/`fW` are budget slices from the
ray and work ceilings. `measured restart` is the real counter for completing the
whole 24 000-slot volume (`update` calls until `complete()`) at the same radius.

| Scene | Place | R | cells | slots | touched | rays_for | rays ceiling | fR | work ceiling | fW | measured restart (frames / rays / work) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Empty | edge | 1 | 150 | 900 | 12 | 6 | 192 | 1 | 990 | 1 | 3 / 96 / 24090 |
| Empty | edge | 4 | 1320 | 7920 | 12 | 6 | 192 | 1 | 8010 | 1 | 3 / 96 / 24090 |
| Empty | edge | 24 | 4000 | 24000 | 12 | 6 | 192 | 1 | 24090 | 3 | 3 / 96 / 24090 |
| Empty | center | 4 | 3240 | 19440 | 12 | 6 | 192 | 1 | 19530 | 3 | 3 / 96 / 24090 |
| Sparse | edge | 1 | 150 | 900 | 81 | 75 | 2400 | 3 | 2025 | 1 | 15 / 14598 / 37634 |
| Sparse | edge | 4 | 1320 | 7920 | 301 | 295 | 9440 | 10 | 12345 | 2 | 15 / 14737 / 37634 |
| Sparse | edge | 8 | 3800 | 22800 | 852 | 846 | 27072 | 27 | 35490 | 5 | 15 / 14737 / 37634 |
| Sparse | edge | 24 | 4000 | 24000 | 914 | 908 | 29056 | 29 | 37620 | 5 | 15 / 14737 / 37634 |
| Sparse | center | 4 | 3240 | 19440 | 718 | 712 | 22784 | 23 | 30120 | 4 | 15 / 14738 / 37634 |
| Dense | edge | 1 | 150 | 900 | 151 | 143 | 4576 | 5 | 3045 | 1 | 32 / 27844 / 49291 |
| Dense | edge | 4 | 1320 | 7920 | 583 | 575 | 18400 | 18 | 16545 | 3 | 32 / 31741 / 49291 |
| Dense | edge | 8 | 3800 | 22800 | 1626 | 1618 | 51776 | 51 | 47070 | 6 | 32 / 31741 / 49291 |
| Dense | edge | 24 | 4000 | 24000 | 1688 | 1680 | 53760 | 53 | 49200 | 7 | 32 / 31741 / 49291 |
| Dense | center | 4 | 3240 | 19440 | 1340 | 1332 | 42624 | 42 | 39420 | 5 | 32 / 31740 / 49291 |

The whole-volume union (`touched`) is 12 (empty), 914 (sparse) and 1688 (dense);
the measured restarts are 3, 15 and 32 slices respectively. The 53-slice
whole-volume ceiling (dense) is the review's slot-count arithmetic with two rays
per sample; the shipped loop only traces rays for exposed slots and its measured
hit rate is about 1.0 (sparse) to 1.2 (dense) rays per sample, which is why the
real restart is 32.

### Counterexample: the `R`-only expansion misses a sun occluder

Fixture: floor, a one-cell receiver at `[4,1,8]` whose `+Z` face gathers the
floor about 1.6 m ahead, and the moving body at `[3,1,14]` (before) /
`[3,1,13]` (after). `R = 4`.

```
[probe] counterexample R=4 receiver=[4, 1, 8] face=+Z body_before=[3, 1, 14] body_after=[3, 1, 13]
[probe] counterexample r_only_radius=4 conservative_radius=9 receiver_chebyshev_before=6 receiver_chebyshev_after=5 origin_distance_before=5.025 origin_distance_after=4.031
[probe] counterexample value before=[0.06618919, 0.06618919, 0.06618919] after=[0.05515766, 0.05515766, 0.05515766] delta=0.011032 reversed before=[0.0330946, 0.0330946, 0.0330946] reversed after=[0.0330946, 0.0330946, 0.0330946]
[probe] counterexample changed_faces=16 outside_conservative=0 outside_r_only=[([4, 1, 8], 1), ([4, 1, 8], 4)] (count 2)
```

- The body's own cells are `6` and `5` Chebyshev cells from the receiver, so a
  tracker that expands by `ceil(R) = 4` never marks it; the conservative `9`
  does. The receiver's sample origin is a lower-bound `4.031 m` from the nearest
  body cell (`5.025 m` for the far placement), so no gather ray of `R = 4` can
  reach either body cell: only the hit-to-sun segment can see the change.
- The completed volumes differ by `0.011032`, exactly one 16-sample
  contribution (`0.9 * 0.196116 / 16`), i.e. one quadrature sample lost its
  sunlit floor hit.
- Control: with the sun's horizontal component reversed the two volumes are
  bit-identical at the receiver (`0.0330946`), which is what a change in sun
  visibility and not in gather geometry requires.
- All 16 changed faces are inside the conservative bound (0 omissions); 2 are
  outside the `R`-only bound.

### Fresh-volume differences against the conservative bound

For every scene, both body positions and all five radii (30 pairs, 60 completed
volumes), every face whose published value changed was inside the
`ceil(2R) + 1` bound: **0 omissions**. Changed faces outside the `ceil(R)` bound:
8 combinations had 0, dense mid-box had 6 at `R = 2` and 8 at `R = 4`, the
counterexample had 2. Every changed face is exposed in one of the two
placements, and the empty scene changed no face at all (a lone body in a void
carries no radiance).

Actual changed-value counts against the conservative `touched` count at `R = 4`:
sparse edge 13/301 (4.3 %), sparse center 13/718 (1.8 %), dense edge 24/583
(4.1 %), dense center 50/1340 (3.7 %).

### Verification (this source)

- Probe: `cargo test --locked -p matterweave-render --lib
  moving_lighting_probe -- --nocapture`
  -> `2 passed; 0 failed; 107 filtered out` in 0.46 s, 76 `[probe]` lines.
- Full render lib suite: `cargo test --locked -p matterweave-render --lib`
  -> `109 passed; 0 failed; 0 ignored` (107 pre-existing plus 2 new).
- Scoped clippy: `cargo clippy --locked -p matterweave-render --all-targets --
  -D warnings` -> exit `0` (only the pre-existing `vendor/winit` notice).
- `cargo fmt -p matterweave-render -- --check` -> clean.
- `python3 tools/check_docs.py` -> `PASS: 222 Markdown files, 655 local links,
  16 ADRs and 20 requirements`, exit `0` (includes this log).

| Criterion | Verification method | Owner | Result |
| --- | --- | --- | --- |
| Dependency counterexample | Actual GI update proves the `R` bound misses a sun-occluder case | this worker | **PASS (host)** — `[4,1,8] +Z` changes `0.06618919 -> 0.05515766` at Chebyshev 5/6 with origin distance 4.031/5.025 > `R = 4`; reversed-sun control bit-identical; 2 changed faces outside `ceil(R)`, 0 outside `ceil(2R)+1` |
| Feasibility table | Deterministic fixtures, face/ray/work counts, correct ceiling bounds | this worker | **PASS (host)** — 3 scenes x 2 positions x 5 radii; 0 omissions across 30 volume pairs; counts in the table above |
| Production unchanged | Scoped experimental file diff, quality 24 preserved | this worker | **PASS** — test-only module + `#[cfg(test)]` registration + this log; `GATHER_DISTANCE_M = 24.0`, `UPDATE_BUDGET`, `indirect.rs`, shaders, app and STATUS untouched |
| Delivery | Focused checks, log, commit/push/PR, frozen review SHA | this worker | **PASS** — see Delivery snapshot |
| Independent review | Opus medium reviews proof + next recommendation | Codex dispatch | **NOT RUN** — outside this writer's scope |

## Insights

- **The two-segment radius is confirmed by construction.** The `R`-only
  expansion is not merely risky: with `R = 4` a completed volume changes by an
  exactly one-sample amount at a face whose cell and origin are both farther
  than `R` from the moved body, and reversing the sun removes the change
  bit-for-bit. `ceil(2R) + 1` is the radius that covered every measured change.
- **The bound is position-dependent, and the review's arithmetic was coarse in
  both directions.** At `R = 4` the conservative set is 1320 cells (33 % of the
  box, 7920 slots) near the edge but 3240 cells (81 %, 19 440 slots) mid-box;
  it reaches the whole box at `R = 8` mid-box and at `R = 24` everywhere. The
  reviewer's `(2d+2)^3` bounding box therefore overstates the edge case, while
  the practical reading (no saving worth having) still holds. At the production
  distance the correct bound *is* the whole volume, so invalidation granularity
  cannot reduce work at `R = 24` at all.
- **The reviewer's 750-frame re-key estimate is a ceiling, not the shipped
  loop.** Only exposed slots trace rays (908 of 24 000 slots sparse, 1680 dense),
  so a completed volume takes 15 and 32 budget slices, and the empty scene takes
  3 (work-bound). That is still far more than one slice per one-cell-per-frame
  move, so the outage conclusion survives, but the number to plan against is
  15/32, not 750.
- **A correct `R = 4` dirty set does not close the motion gap.** It would
  resample 33-34 % of the exposed faces near the edge and 78-79 % mid-box
  (`rays_for` 295/908, 575/1680, 712/908, 1332/1680), with ray ceilings of
  10-18 slices (edge) and 23-42 (mid-box). With a body moving one cell per
  frame, `complete()`-gated publication still starves: partial publication is
  the gate that matters, not the dirty radius.
- **No universal frame claim exists.** The empty scene's `R = 4` conservative
  bound costs 8010 work units at the edge position, inside one 8192-unit slice
  (mid-box 19 530, three slices); the falsifiable statement is per-fixture,
  which is why the probe prints all of them.
- **Exact dependency tracking has headroom but no implementation.**
  Changed-value counts are 1.8-4.3 % of the conservative touched slots
  (13-50 vs 301-1340), so a per-slot dependency map could shrink dirty sets by
  roughly 20-50x on these fixtures. Nothing in the engine maps a changed cell to
  the slots whose rays crossed it; this is an unmeasured hypothesis, not a
  result, and this probe does not implement it.
- **A future dirty set must zero as well as recompute.** `IndirectVolume::update`
  writes a slot only when its sample loop completes and otherwise leaves the
  stored value; a dirty-set update on top of persistent values must explicitly
  zero slots that become covered. The current whole-volume `clear()` hides that.
  No incremental implementation exists to test, so this is stated as a hazard,
  not a measured defect.

## Next implementation recommendation

**Result-backed (host evidence above):** do not implement `ceil(R)`
dependency-bounded invalidation, and do not expect invalidation granularity to
make GI live during motion. The correct radius is `ceil(2R) + 1`; at the
rejected `R = 4` a correct dirty set still needs 10-42 budget slices by its
ceilings (15/32 measured for the full restart), and at the production `R = 24`
it covers the whole box.

**Recommendation:** the next implementation slice should be **partial
publication** — a per-slot (or per-region) validity state on the published
volume plus the consumer/shader contract for reading a partially valid volume —
because that is the only change measured here that removes the `complete()` gate
that turns a 15-32-slice reconvergence into a GI outage. A correct dirty set, if
added later, is a bounded complement to it and must use `ceil(2R) + 1`, never
`ceil(R)`.

**Hypothesis, not result (unimplemented, unmeasured):** exact per-slot
dependency tracking (ray-path -> slot indexing) could cut the dirty set by the
1.8-4.3 % ratio above; the engine has no such index and the probe did not build
one.

## Open gates (not run, not claimed)

Independent Opus review of the frozen source; any Android/device behaviour; any
wall-clock, thermal or frame-rate measurement; adoption of a bounded gather
distance; a dirty-set or partial-publication implementation. This probe measures
functional counts only and completes no GI work item.
