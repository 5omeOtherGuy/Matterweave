# PR #51 review repair — packed-occupancy traversal experiment (issue 42)

Repair worker log. Reviewed source: `75a8268db6c78907cb06d2973dded466bc59f59b` on branch
`engine/ray-hierarchy-experiment`. Review input:
`../orchestration/opus-ray51-review/result.json` (Opus, read-only, static inspection).
Repair scope: the new crate `crates/matterweave-ray-hierarchy`, the experiment log
`docs/performance/logs/ray-hierarchy-experiment.md`, and this file. No shared renderer,
API, `STATUS`/`ROADMAP`/board edit is part of this repair; one shared-renderer change is
proposed separately below. Issue 42 stays open.

**State: review findings addressed with code and evidence on CPU. GPU and Android
execution NOT RUN. No adoption, no merge, no performance claim.**

## Finding-by-finding disposition

Verified each review finding against the actual source before acting; two review
recommendations were adjusted because the source and the expanded corpus contradicted
them. Details and commands are under **Actions Taken** and **Solutions Applied**.

| # | Review finding | Verification against source | Disposition |
| --- | --- | --- | --- |
| 1 | The "cached block word" is not implemented; counters and the adoption decision model an optimisation the code does not perform | Confirmed: `traverse.rs` set `cached_block` and incremented `occupancy_word_loads` once, then called `grid.block_occupied` (ORs every word) and `grid.cell_occupied` (another word) per cell | **Fixed.** Real cache: `cached_words: [u32; MAX_BLOCK_BITS/32]` + `cached_any`, refilled only when the block index changes, every block/bits test answered from it, `occupancy_word_loads` counts `words_per_block` per refill. Counter table, reading text and adoption wording re-recorded |
| 2 | Tie classifier accepts any disagreement within a ±1 cell box, never compares material/normal, never fails on a real off-by-one; 2% budget hides it | Confirmed (`differential_tests.rs` old `is_tied_plane_artifact`: `differing > 0`, distance-only, no material check) | **Fixed, adjusted.** The exact count is pinned (no percentage budget) and the classifier is geometry-anchored: same material and distance, every differing cell one step away **with the hit point on the plane the two cells share**, and each normal must name a face of its own cell containing that point. The review's `differing == 1` was **not** adopted as-is: the expanded corpus produces a legitimate corner tie (2 differing axes) that the line rule would reject as an error. Negative controls added |
| 3 | Entry routine has no independent-oracle evidence; `StartKind::Outside` only compared modes | Confirmed: seeded oracle corpus used `Center`/`Quarter` only; the 2,304-ray mode corpus is vacuous for entry semantics because all modes share `Walk::new` | **Fixed.** The seeded oracle corpus now runs all four start classes (half of them `Integer`/`Outside`). It immediately found a hit/miss difference at an exact crop-exit tie; that class is classified, counted and pinned, not excused |
| 4 | Provenance claim that "no source was copied" implies "no licence obligation" states a legal conclusion the record does not support | Confirmed at experiment-log lines about the DeadlockCode row and the friction section | **Fixed.** Reworded to facts (file read at the pinned revision, dual MIT/Apache-2.0 confirmed via `gh api`, packing and gated read reimplemented, bit order and all code written here, no upstream text copied) plus an explicit open item for the owner's licence decision. No licence selected |
| 5 | `max_distance` domain deviates from `World::raycast` (`<= 0.0` and `> cap` rejected) and is undisclosed | Confirmed: `traverse.rs` rejected `0.0` and values above the cap; the log listed only the direction deviation as new | **Fixed.** Finite ranges above `MAX_RAY_DISTANCE` are clamped and `max_distance == 0` from inside the crop answers the inside-solid query, both matching `World::raycast`; the shader's zero-length-segment discard is recorded as the one place the CPU oracle defines the query. Differential test added |
| 6 | Scope item: `MAX_COORD` duplicated in `volume.rs` and `ray_reference.rs`; validator should be exported from the reference | Confirmed the literal `8192` exists in both crates; `ray_reference` exports `MAX_CELLS`/`MAX_AXIS` but no crop validator or `MAX_COORD` | **Proposed separately**, not applied: it is a shared-renderer API change and the task forbids those without a concrete need. Text below |
| 7 | Minor: `trace()` discards `stats.exhausted`; `block_index(block)?` turns internal inconsistency into a silent miss | Confirmed | **Addressed cheaply.** `trace()` documents the loss and points at `trace_stats`; the block lookup now `debug_assert!(false, ...)` before returning the miss, so tests fail loudly |

## Actions Taken

1. Read the review result in full and re-read every cited source region
   (`traverse.rs`, `occupancy.rs`, `differential_tests.rs`, `traversal_tests.rs`,
   `matterweave-core/src/ray.rs`, `ray_reference.wgsl`) before editing.
2. Implemented the block cache and rewired the walk to it; deleted the counter-only
   `cached_block` behaviour. `block_occupied`/`cell_occupied` on `OccupancyGrid` remain for
   mutation and tests; the walk reads `grid.block_words` only inside `refill_block`.
3. Rewrote `is_tied_plane_artifact` (geometry-anchored) and added a positive/negative
   control test with constructed hits: material change, diagonal neighbour with the point
   inside the cells, two-cell jump, distance/face mismatch, non-axis normal, wrong-face
   normal — all rejected; face tie, corner tie and same-cell different-face tie accepted.
4. Replaced the 2% artifact budget with exact pinned counts and printed enumerations.
5. Extended the seeded oracle corpus to `Center`/`Integer`/`Quarter`/`Outside` and
   classified the new divergence classes it exposed.
6. Adopted `World::raycast`'s `max_distance` domain and added an oracle-parity test.
7. Updated code docs (`lib.rs`, `traverse.rs`, `volume.rs`, `README.md`) and the experiment
   log: provenance wording, the BlockMask model, the counter table, the results table, the
   tie enumerations, the max-distance semantics, the decision text and the STATUS/ROADMAP
   snippets.
8. Re-ran focused checks only (see **Verification performed**); no release build, no
   workspace-wide rebuild, no other worker's target directory, `/mnt/bench` never touched.
9. Corrected the PR #51 body (counters, provenance, tie and crop-boundary evidence) and
   pushed; the frozen SHA is reported in the PR/handoff, not inferred here.

## Issues & Friction

- **The review's Finding 1 premise was exact.** The old test
  `cached_block_avoids_repeated_occupancy_word_loads` asserted `occupancy_word_loads == 1`
  while the traversal re-read the block words per cell; the counter and the test agreed
  with each other and neither agreed with the memory access pattern.
- **The review's `differing == 1` classifier rule did not survive contact with the
  expanded corpus.** With origins outside the crop and exact integer origins included, the
  corpus produced `candidate [5, 14, 2] / oracle [5, 13, 1]`, both material 85, both
  `d = 2.4874685`, normal `[1,0,0]`: a two-axis difference where the hit point
  `(6, 14, 2)` is the shared corner and both cells are the same voxel material. That is a
  legitimate tie, so the classifier verifies the geometry (point on every shared plane,
  each normal naming a face that contains the point) instead of counting differing axes.
  This is stricter than the proposed rule for arbitrary errors and still accepts the
  proven corner case.
- **Expanding the corpus found two more real divergence classes**, both traced by hand
  before classification rather than absorbed into the budget:
  a same-cell/different-entry-face tie (`[5, 0, 8]`, normals `[-1,0,0]` vs `[0,1,0]`, same
  point on the cell's corner) and an exact tie on the crop's exit face (seed 34, index 9:
  candidate steps x, y and z together and leaves the crop at z = -10, the accumulated
  oracle steps into the in-crop corner cell `[9, 3, -11]`). Both are the disclosed
  accumulated-vs-recomputed class; the second is the crop-boundary rule seen from the exit
  side.
- **Isolating the exit-face case needed a temporary ignored debug test** that replayed the
  corpus generator and printed the seed (seed 34, index 9). It was deleted before commit;
  it is not part of the crate.
- **Ordering hazard:** the `max_distance == 0` carve-out needs `starts_inside`, which the
  original code computed after the zero-length guard; the guard was moved after it.
- **Clippy:** three `needless_range_loop` errors from the new classifier loops were fixed.
  The vendored `winit` `function_casts_as_integer` warning pre-exists this work and is not
  in the new crate.
- **No friction with the reference lineage:** the retained reference tests still pass
  unchanged; no renderer file was read-modify-written.

## Decisions & Rationale

1. **Implement the cache (review option 1), not "count honestly and restate".** The
   adoption decision, the counter table and the proposed GPU port all rest on
   one-block-fetch-per-entry; leaving the claim while removing it would have made the
   candidate strictly worse than documented. Cost is bounded: one `[u32; 16]` array in the
   walk and a `copy_from_slice` per block entry.
2. **Physical reads are whole blocks.** A refill fetches `words_per_block` words and counts
   exactly that; empty-block words are still fetched, so an empty block costs one block
   fetch, not zero. No cache-line or bandwidth claim is made.
3. **Geometry-anchored tie acceptance, exact pinned counts.** The classifier fails closed:
   anything not explained by material + distance + shared-plane point + face-consistent
   normal is an error. Counts are pinned exactly (3 tied-plane resolutions, 4
   crop-boundary ties) and printed, following the targeted corpus's `assert_eq!(..., 0)`.
4. **Crop-boundary differences are classified, not excused.** The predicate requires the
   oracle hit to be exactly at the crop's exit parameter and on the hit cell's boundary,
   so an unrelated miss cannot hide behind it.
5. **Adopt the oracle's `max_distance` domain.** Clamping is unambiguous; `0` answers the
   inside-solid query. The shader cannot express a zero-length segment, so the crate's
   semantics for that one query come from `World::raycast` and this is stated in both
   `lib.rs` and the log.
6. **Do not touch shared renderer files.** The `MAX_COORD` validator export is proposed as
   its own change so the integration worker can sequence it.
7. **No adoption claim moved.** `BlockMask` remains a candidate for the later device gate;
   the counter change does not create a measured win.

## Solutions Applied

- `crates/matterweave-ray-hierarchy/src/traverse.rs`: real block cache (`cached_words`,
  `cached_any`, `refill_block`, cached `cell_occupied`), `words_per_block` physical-read
  counting, `max_distance` clamp + zero-length inside-solid query, documented `trace()`
  exhaust flag, `debug_assert!` on the impossible block lookup.
- `crates/matterweave-ray-hierarchy/src/differential_tests.rs`: geometry-anchored
  `is_tied_plane_artifact` and `normal_names_face`, strict `is_crop_boundary_deviation`,
  exact pinned artifact counts, all four start classes in the seeded oracle corpus,
  `tied_plane_classifier_rejects_unexplained_disagreements` negative controls,
  `max_distance_domain_matches_the_world_oracle`.
- `crates/matterweave-ray-hierarchy/src/traversal_tests.rs`: whole-block fetch test
  (`cached_block_words_are_fetched_once_per_block_entry`), zero-length/clamped-range oracle
  test, per-shape `loads == entries * words_per_block` accounting, updated rejection matrix.
- `crates/matterweave-ray-hierarchy/{src/lib.rs,src/volume.rs,README.md}` and
  `docs/performance/logs/ray-hierarchy-experiment.md`: semantics, provenance and counter
  corrections; tie and crop-boundary enumerations.
- Result: 32 tests pass, clippy and fmt clean, retained-reference tests pass, docs check
  passes.

## Verification performed

Environment: recovery SSD, `RUSTC_WRAPPER=` (no inherited sccache), `CARGO_BUILD_JOBS=1`,
`CARGO_TARGET_DIR=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/ray-experiment-target`.
`/mnt/bench` not used. 18-19 GB free before and after. No release build, no workspace-wide
rebuild; only the new crate plus the retained reference's focused test target.

```sh
cargo test -p matterweave-ray-hierarchy --lib            # 32 passed, 0 failed
cargo clippy -p matterweave-ray-hierarchy --all-targets -- -D warnings   # no errors in this crate
cargo fmt -p matterweave-ray-hierarchy -- --check        # clean
cargo test -p matterweave-render --lib ray_reference     # 7 passed (retained reference intact)
python3 tools/check_docs.py                              # PASS: 219 files, 651 links, 16 ADRs
```

Printed corpus output (`--nocapture`), recorded because the review asked for counted
facts rather than summaries:

```text
targeted oracle cases: 96 hits, 36 misses, max delta 0, 0 ties, 0 crop-boundary ties
block boundary oracle cases: 20 hits, 36 misses, max delta 0, 0 ties, 0 crop-boundary ties
seeded oracle corpus: 213 hits, 548 misses, max delta 0, 3 tied-plane resolutions, 4 crop-boundary ties
mode corpus: 743 hits, 1561 misses, 1201 coarse steps (2304 rays, 3 shapes)
max-distance domain: 15 hits, 3 misses, max delta 0, 0 ties, 0 crop-boundary ties
depth comparison: 36 hits
```

Corrected structural counters over the 2304-ray mixed-shape corpus (buffer accesses, not
timings). Only the two occupancy-word columns changed relative to the reviewed log:

| Mode | iterations | fine steps | block steps | catch-up planes | blocks checked | block word loads | cells examined | material reads |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `Reference` | 11581 | 9649 | 0 | 0 | 0 | 0 | 10392 | 10392 |
| `BlockMask` | 11581 | 9649 | 0 | 0 | 10392 | 22766 | 6021 | 743 |
| `BlockStep` | 8411 | 5278 | 1201 | 3101 | 7435 | 22766 | 6021 | 743 |

The mode corpus mixes three block shapes (2, 4 and 16 words per block), so the total does
not divide by one shape's word count; `block_shape_scales_occupancy_cost_not_hits` pins
`occupancy_word_loads == block entries * words_per_block` per shape and asserts monotone
block entries across nested shapes. Previous reviewed figures (3486 "loads") counted block
*entries* as one load each while the code re-read every word; they are superseded.

## Definition of done

| Criterion | Verification method | Owner | Result |
| --- | --- | --- | --- |
| Existing solution assessment | Scoped source/reference/provenance notes, explicit adoption decision | You | **PASS.** Review-driven re-verification of every claim; provenance reworded to facts with the licence decision left open; `BlockMask` still a candidate, `BlockStep` deferred |
| Executable optional candidate | Actual implementation with bounded data/update contract | You | **PASS.** Real block cache in the walk; bounded build/patch/iteration contracts unchanged and re-tested; 32 tests pass |
| Correctness | Seeded and targeted differential tests, exact hit/material/depth semantics | You | **PASS on CPU with enumerated exceptions.** 974 oracle comparisons, max distance delta 0; 3 tied-plane resolutions and 4 crop-boundary ties pinned, printed and geometrically classified; 2304 mode rays bit-for-bit equal. The exceptions are disclosed, not hidden |
| Delivery | Focused checks, inspected diff, code/log committed+pushed, reviewable PR | You | **PASS pending push/PR update in this same turn**; frozen SHA reported in the PR body and handoff. No merge, no issue closure |
| Independent review | Frozen-source review requested from Codex after handoff | Separate reviewer | **NOT RUN** — this turn only repairs the previous review; a follow-up review of the new SHA is requested by the task owner |
| Android/cost acceptance | Explicit next gate, no unrun pass or performance inference | Later device owner | **NOT RUN** — GPU/Android execution, mobile precision, representative-quality cost, thermal behavior all remain unmeasured; no performance inference is made |

## Insights

- A counter-only "optimisation" can pass its own test: the old test asserted the counter's
  value, so it verified the counter, not the memory access. The repair makes the test assert
  the physical fetch (`words_per_block` per entry) and the walk reads only the cache.
- Entry clipping is exactly where hand-picked in-crop rays are weakest: the expanded corpus
  found the exit-face tie that four targeted rays and 2304 reference-free mode rays did not.
- Recomputing plane crossings and accumulating them are both "correct" f64 algorithms and
  disagree only at exact ties; the candidate matches the reference shader's simultaneous
  stepping, so the disagreements are oracle-side float artefacts and are legitimate only
  when the geometry proves the same hit point. The classifier now encodes that proof.
- The zero-length-segment rule is the reference shader's; the CPU oracle accepts a
  zero-length query. The crate follows the oracle there and says so, which keeps the oracle
  differential honest without pretending the shader agrees.
- The remaining cost question is cache behaviour, which these counters cannot answer. The
  honest statement is: fewer material words, more occupancy words, one fetch per block
  entry — measure on the device.

## Proposed updates for the integration worker (NOT APPLIED)

`docs/STATUS.md`, new bullet in the current performance/experiment section:

```text
- Bounded hierarchical traversal experiment (issue 42): new workspace member
  crates/matterweave-ray-hierarchy with packed 4x4x4 / 4x4x8 / 8x8x8 block occupancy
  beside dense materials. CPU differential evidence: 974 World::raycast comparisons (132
  targeted, 56 block-boundary, 768 seeded over four start classes including out-of-crop
  origins, 18 max-distance domain) with no measured distance deviation and every one of
  the 3 tied-plane resolutions and 4 crop-boundary ties enumerated and geometrically
  classified; bit-for-bit equality between the reference, block-mask and coarse-step modes
  over 2304 seeded rays; bounded build/patch accounting and explicit staleness. GPU and
  Android execution NOT RUN; no performance claim; ADR-0006 unchanged.
  Logs: docs/performance/logs/ray-hierarchy-experiment.md,
  docs/performance/logs/ray-hierarchy-review-repair.md.
```

`docs/ROADMAP.md`, M2 traversal line: note the experiment as a reviewed CPU candidate
awaiting device functional execution and the representative cost comparison; keep the
DAG/tree decision open.

`docs/performance/board.json`, new task entry (state advanced to the follow-up review):

```json
"RAY-HIERARCHY": {
  "attempt": 2,
  "owner": "ray-hierarchy-experiment",
  "state": "review-requested",
  "artifact": "crates/matterweave-ray-hierarchy",
  "log": "docs/performance/logs/ray-hierarchy-review-repair.md"
}
```

## Proposed separate change (NOT APPLIED)

Export the crop validator from the retained reference so the duplicated literal disappears:

```diff
 // crates/matterweave-render/src/ray_reference.rs
+/// Validates crop bounds exactly as `RayVolume::pack` does, for callers that only need a
+/// yes/no answer (the hierarchy snapshot reuses it instead of a second literal).
+pub fn validate_crop(origin: [i32; 3], dimensions: [u32; 3]) -> Result<usize, String> {
+    // body shared with `pack`: MAX_AXIS, MAX_CELLS and the +/-8192 endpoint bound
+}
```

`crates/matterweave-ray-hierarchy/src/volume.rs` would drop `MAX_COORD` and call it. This
is a `matterweave-render` API change, so it is deliberately not part of this repair; open
it as its own PR and keep the ten-row acceptance test meanwhile.

## Next gate for the device owner

1. Port the block-mask walk into a new `matterweave-render` module + fragment kernel behind
   the renderer's ownership (proposed diff in the experiment log; not applied here).
2. Reuse `RayUniform` and add the occupancy-word storage binding; keep the stateless
   per-plane formula and the simultaneous tie rule, and implement the `max_distance == 0`
   inside-solid query explicitly if the port wants oracle parity.
3. Compare readback against `World::raycast` for the crate's targeted cases, the four start
   classes and the 4 pinned crop-boundary rays under `VK_LAYER_KHRONOS_validation` with
   synchronization validation.
4. Only then run the representative-quality cost comparison from issue 42 on the phone,
   recording device, OS, driver, scene, seed and build configuration. f64 host agreement
   does not transfer to mobile f32; the retained reference already needed explicit Adreno
   tolerances.

## Limits

- Everything here is CPU-only, host-f64, single-threaded and structural. No wall-clock,
  GPU, thermal or representative-quality claim is made or implied.
- The cache is one block, refilled on block change; it is not a multi-level hierarchy and
  it does not model upload, streaming or compressed residency.
- The classifier's tolerance is geometric (`1e-6` relative), not a proof over all float
  inputs; it fails closed on anything it cannot explain, which is the intended direction.
- The corpus is deterministic but not exhaustive; the pinned counts move only with a
  deliberate corpus change and must be re-enumerated in this log if they do.
