# Exact dependency-based face retention for mesh-proxy edits (issue 47) — 2026-09-13

Bounded, opt-in, exact dependency retention for `IndirectVolume` proxy edits, plus
the wetland consumer that opts in. It is the next functional moving-lighting slice
after the source-backed rejections recorded in
`orchestration/deepseek-moving-lighting-probe/result.json`,
`orchestration/opus-moving-probe57-review/result.json` and
`orchestration/opus-moving-lighting-feasibility/result.json` (outside this
repository, checked with the probe log's own citations):
the `ceil(R)` radius is wrong, a correct `ceil(2R)+1` radius is the whole box at
`R = 24`, partial publication alone publishes a mostly-neutral volume, and exact
per-slot dependency tracking was an unmeasured hypothesis. This slice implements
that hypothesis with an unchanged complete-only publication contract.

**Scope.** `crates/matterweave-render/src/indirect.rs` plus the new submodule
`crates/matterweave-render/src/indirect/dependency.rs` (registered from
`indirect.rs`, not `lib.rs`), `apps/explorer/src/wetland_lighting.rs`
(attach/config/report and its tests) and this log. No shader, `lib.rs`,
`world.wgsl`, budget, gather-distance or sample-count change; no new publication
mode; no stale or partial lighting is ever uploaded. Host-only: no device, no
phone, no `/mnt/bench`, no wall-clock and no thermal claim.

## Actions Taken

- Read the three orchestration results and checked each load-bearing claim
  against the frozen `indirect.rs`: one sample traces two segments
  (`trace_scene` gather, then `trace_scene` from the hit point to the sun, both
  capped at `self.distance = GATHER_DISTANCE_M = 24.0`), `set_mesh_proxy`
  unconditionally `clear()`s, `complete()`/`valid_for`/`source_valid` gate every
  publication, and `update` re-keys on `mesh_digest()`.
- Added `crates/matterweave-render/src/indirect/dependency.rs`: per-face proxy
  cell bitsets over the volume's own coverage box, a DDA reader that mirrors
  `matterweave_core::World::raycast`'s cell reads, the old/new proxy diff, and
  the invalidation rules. 13 tests in the same file.
- Extended `IndirectVolume`:
  `enable_proxy_retention(max_bytes) -> Result<RetentionStatus, String>`,
  `replace_mesh_proxy(Option<MeshProxy>) -> ProxyEdit`, `retention_status()`,
  `dirty_faces()`; tightened `pending_work()`; added a per-face `done` bitmap so
  the update scan counts work per *unresolved* face instead of per box slot.
  `set_mesh_proxy` keeps its clear-all semantics byte for byte.
- Integrated the consumer: the wetland volume opts in with a 6 MiB cap and
  replaces its proxy through `replace_mesh_proxy`; the report line now carries
  `retained`, `invalidated`, `dirty` and `dependency_kib`. The reviewed-54 error
  latch, reflection-stale logic, digest gate, withdrawal on a changed footprint
  and complete-only upload are unchanged.
- Ran the render lib suite, the full explorer lib suite, scoped clippy with
  `-D warnings`, `rustfmt --check` and `tools/check_docs.py`.

## Issues & Friction

- **The first version still paid box-sized `work` per edit.** The scan skipped
  retained faces for free but counted a work unit for every never-sampled face,
  so a 65-face invalidation still needed three 8192-work slices and `max
  frames/move` stayed at 3 (work ratio 0.64 of whole-volume). Fixed by an
  explicit per-face `done` bitmap (3 KiB): a face is done once it is sampled
  *or* found enclosed, and retention clears `done` for exactly the faces an edit
  can affect. Measured work ratio dropped to 0.039 (sparse) / 0.034 (dense).
- **The recorder initially walked past the proxy's coverage box.** The walker
  was given the trace's limit (up to 24 m) while `MeshProxy::raycast` clips to
  `segment_exit`, so the recorder walked cells the DDA never reads: 424 583
  "outside" cell visits in the sparse fixture, all ignored. Harmless for
  correctness (out-of-box cells cannot change under the coverage rule) but real
  wasted work. The walker is now clipped with `proxy.segment_exit`, the same
  value the traced DDA uses; outside visits fell to 3 879 (the boundary-face
  origins) with identical values and identical retention counts.
- **One app expectation changed, and it is an improvement rather than a
  regression.** `the_proxy_footprint_gates_invalidation` asserted that a
  cell-crossing move leaves `indirect_live == false`. With retention the
  invalidated set of that fixture drains inside one `UPDATE_BUDGET`, so the
  withdrawn publication is replaced by a complete, current one in the same
  frame. The test now asserts: the previous publication is retired
  (`withdraw_calls` rises), at least one face was invalidated, and a live
  indirect state is a *new* upload with `pending_work == 0` (the `FakeSink`
  re-checks `complete()`, `valid_for`, digest match and non-retired digest).
  See "Behavior delta vs the frozen 54" below.
- **`MeshProxy` is not `Clone` and the volume's proxy is private**, so a test
  cannot inspect the retained proxy directly; tests observe retention through
  `ProxyEdit` counts, `sample()` and the volume's own `done`/`values` state
  (visible to the submodule's tests). No production state was exposed for tests.
- **The `dependency.rs` test module is large** (1 250 of the file's 1 816
  lines). Kept in the owned new file rather than `indirect_tests.rs` because the
  brief restricts the new file scope and forbids other renderer edits.

## Decisions & Rationale

- **Index space = the volume's own coverage box.** `IndirectVolume::new` caps
  face slots at 24 576, so a box is at most 4 096 cells and one face bitset is
  at most `ceil(4096/64)*8 = 512` bytes. A proxy whose *changed* cells fall
  outside that space is a coverage change and takes the old clear-all path
  (`tracked = false`), which is the conservative branch the brief allows. A
  proxy box that is merely *larger* while its occupied cells stay inside remains
  exactly trackable and is tested.
- **Only proxy cells are recorded.** `World` cells are fixed by the key
  (`epoch`, world `revision`, sun, mesh digest), and any change to them clears
  the volume in `update`, so recording them would be redundant. `World` exposes
  no cell visitor, so `walk_segment` mirrors the shipped DDA's reads; the mirror
  is pinned by a test that compares the visited terminal cell and the traversal
  against `World::raycast` on identical cells and against
  `MeshProxy::raycast` on the same segments (11 000+ segments, 100+ hits).
- **Record empty traversed cells and the first hit.** A removed first hit must
  invalidate (it is recorded), and an occluder inserted on a previously empty
  path must invalidate (the empty cells are recorded). Cells behind the first
  hit are not recorded; a change there first has to pass the recorded hit cell,
  which invalidates the face.
- **Exposure dependencies are handled twice on purpose.** A sampled face records
  its own cell and outward neighbor with its ray cells, and the edit path
  independently expands every changed cell over its closed seven-cell
  neighborhood. The second rule is what catches faces that carry no bitset or a
  partial one; the first keeps a face's own record self-contained.
- **Retention is never a partial publication.** The volume still publishes only
  a complete key for one source identity; retention only shortens the recompute
  between `complete()` states. `valid_for`, `source_valid`, `complete()` and the
  upload path are untouched, and every invalidated slot is zeroed immediately
  (value *and* completed marker) so a mid-key read can never see a stale value.
- **The bitset arena is caller-capped and degrades per face.** Bitsets are
  allocated lazily on a face's first sample; a face that does not fit the cap is
  `UNTRACKED` and recomputed on every edit instead of retained. `tracked_faces`,
  `untracked_faces`, `resident_bytes` and `cap_bytes` are reported, and the
  caller's cap is clamped to `MAX_DEPENDENCY_BYTES` (16 MiB). `UNTRACKED` is
  counted as intersecting every edit, so a missing bitset can never retain a
  stale value.
- **Opt-in, not default.** No existing caller pays for tracking: `tracking` is
  `None` until `enable_proxy_retention`, and the untracked path is the previous
  code path (the `done` bitmap only changes which faces count as work, and in the
  untracked path every slot is unresolved exactly as before).
- **Production quadrature and budgets untouched.** `GATHER_DISTANCE_M = 24.0`,
  `SAMPLES = 16` and `UPDATE_BUDGET { rays: 1024, work: 8192 }` are unchanged;
  the tests use those exact values. No `R` is adopted anywhere.

## Solutions Applied

### Source contract

- **Recorded per completed face** (`dependency.rs`): its own cell and outward
  neighbor (exposure), plus every proxy cell read by the gather segment *and*
  the hit-to-sun segment of every one of its samples, including empty traversed
  cells and the first solid hit. Each segment is clipped with
  `MeshProxy::segment_exit(..., limit)` where `limit` is what
  `trace_scene_limit` used (`World` hit distance or `max_distance`), so the
  recorder reads the same cells the DDA reads, in the same order, and a segment
  that never enters the coverage box reads nothing.
- **Invalidation on `replace_mesh_proxy`**: a two-pointer diff of the old and new
  proxy cell lists (ascending `z,y,x`, one pass, no allocation) over
  `(cell, material)` including presence/absence. For each changed cell: its
  closed seven-cell neighborhood's six faces each (exposure). Plus every face
  whose recorded bitset intersects the changed mask, plus every `UNTRACKED` face.
  Invalidated slots are zeroed and their `done` bit cleared; retained slots keep
  their exact value and bitset.
- **Retention preconditions**: tracking enabled, `key.is_some()` (a completed
  source identity exists), and every changed cell inside the index space. Any
  other case (`None` tracking, no key, coverage change) takes `set_mesh_proxy`'s
  clear-all path and reports `tracked = false`.
- **Retained key**: the new proxy digest replaces the mesh component of the
  existing key; `update` still clears everything when `epoch`, world `revision`
  or sun changes, so a retained value can never outlive its source or light.
- **Mid-edit state**: an edit with changed cells discards the in-progress face's
  partial sample loop and partial dependency record (`sample_index`/`sum` reset,
  bitset cleared at the next `begin_face`) and resumes the scan at the earliest
  dirty slot; the accumulated dirty set is not reset, so overlapping edits only
  add invalidation.
- **API compatibility**: `set_mesh_proxy` and `clear` are unchanged in
  behavior; `IndirectVolume::new`, `update`, `complete`, `valid_for`,
  `source_valid`, `sample`, `mesh_digest`, `has_mesh_proxy` and `resident_bytes`
  keep their contracts. `pending_work()` now counts only unresolved slots
  (identical to the previous value in the untracked path, tighter with
  retention).

### Memory accounting (structural, not a benchmark)

| Array | Size for the 20×10×20 wetland box | Notes |
| --- | --- | --- |
| per-face offsets | 24 000 × 4 B = 96 KiB | one `u32` per face slot |
| changed mask | `ceil(4000/64) × 8` = 504 B | per edit |
| per-face `done` bitmap | `ceil(24000/64) × 8` = 3 000 B | always present, untracked path included |
| dependency arena | 504 B × sampled faces (lazily) | capped, `UNTRACKED` beyond |
| measured tracker bytes | 555 320 B (910 faces, sparse) / 945 416 B (1 684, dense) / 127 928 B (58, wetland app fixture) | `RetentionStatus::resident_bytes` |
| app cap | 6 MiB (`INDIRECT_DEPENDENCY_BYTES`) | engine clamp 16 MiB, ~12 000 faces |

A face that cannot be tracked is recomputed on every edit; a cap or allocation
failure therefore degrades retention, never correctness, and no stale value can
survive it (tested).

### Behavior delta vs the frozen 54 (recorded, not hidden)

- **Same-frame republish.** When a changed footprint's invalidated set drains
  inside one `UPDATE_BUDGET`, the wetland scheduler retires the previous
  publication and uploads a *new complete* volume in the same frame instead of
  staying dark. The retire-then-republish sequence, complete-only upload checks
  and the error latch are unchanged; only the dark window shortens. When the
  invalidated set is larger (e.g. a body appears in the detail scene) the
  previous behavior is visible: indirect stays retired until convergence.
- **Proxy-internal only.** The tracker replaces `set_mesh_proxy` only inside
  `attach`, only for this volume; `engine_check.rs`, `mesh_lighting_check.rs`,
  `async_indirect.rs` and every other caller keep the untracked path.

### Test evidence (`RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=../indirect-retention-target`)

- `cargo test --locked -p matterweave-render --lib` → `122 passed; 0 failed`
  (13 new in `indirect::dependency::tests`, the 2 probe tests from PR 57, and
  the pre-existing suite).
- `cargo test --locked -p matterweave-explorer --lib` → `230 passed; 0 failed;
  1 ignored` (16 pre-existing wetland tests + 1 new integration test, plus
  `main`'s newer app tests).
- `cargo clippy --locked -p matterweave-render --all-targets -- -D warnings` →
  exit 0. `cargo clippy --locked -p matterweave-explorer --lib -- -D warnings` →
  exit 0. `cargo fmt -p matterweave-render -p matterweave-explorer -- --check` →
  clean. `python3 tools/check_docs.py` → `PASS: 221 Markdown files, 655 local
  links, 16 ADRs and 20 requirements`.

Correctness tests (all through the shipped `update`/`sample` paths, all 24 000
slots compared bit-for-bit against a fresh untracked volume unless stated):

| Test | What it pins |
| --- | --- |
| `walker_reads_the_cells_the_dda_reads` | the mirrored reader equals the shipped DDA: 11 000+ segments, terminal cell = `MeshProxy::raycast` hit cell, misses pass no solid cell, `World::raycast` on identical cells agrees, unit-step paths, outside-box segments read nothing |
| `far_occluder_second_segment_is_invalidated_exactly` | the probe-57 fixture at `R = 4`: receiver `[4,1,8] +Z`, body `[3,1,14] → [3,1,13]`; the receiver is invalidated by its recorded *sun* segment (5–6 Chebyshev cells away, no gather ray can reach), zeroed mid-key, converges to the fresh volume, and a reversed-sun control is bit-identical |
| `single_cell_edits_match_a_fresh_reference` | move / material / removal / insertion on sparse and dense: bit-exact fresh equality, retained ≫ invalidated, retained slots exact mid-key, `dirty` accounting exact |
| `enclosing_edit_zeroes_the_newly_hidden_face` | an exposure change zeroes the newly enclosed face immediately, `complete`/`valid_for`/`source_valid` refuse it until convergence, then equals fresh |
| `overlapping_edits_accumulate_invalidation` | a second edit one partial slice into the first: invalidation accumulates, no superseded key publishes, final volume equals fresh for the last proxy |
| `world_sun_and_epoch_changes_clear_retained_values` | sun, world revision and replacement epoch each zero every retained value and re-converge to fresh |
| `coverage_change_outside_the_index_space_clears` | an out-of-index changed cell reports `tracked = false`, clears everything, then equals fresh |
| `out_of_box_traversal_cells_do_not_block_retention` | a larger proxy box with in-index changes still retains exactly and equals fresh |
| `tracking_cap_degrades_to_untracked_never_stale` | 8-face cap: `untracked_faces > 0`, retained ≤ tracked, every completed untracked face invalidated, no stale publish, fresh equality; caller cap clamped to `MAX_DEPENDENCY_BYTES` |
| `identical_proxy_edit_keeps_the_complete_volume` | identical footprint: 0 changed cells, 0 invalidations, still complete, `pending_work == 0` |
| `old_set_mesh_proxy_still_clears` | the old API still clears unconditionally and re-converges to fresh |
| `outside_coverage_geometry_is_clipped_from_the_trace` | the coverage box clips the trace: geometry outside it cannot be hit, identical cells give identical traces |

Motion characterization (60 consecutive one-cell moves on a rectangle ring, the
production `UPDATE_BUDGET`, printed by `continuous_motion_retains_most_faces_and_converges`;
"whole" is the untracked clear-all path over the same proxy sequence):

```
[retention] sparse tracking: tracked=910 untracked=0 outside_cells=3879 checked=4000 bytes=555320
[retention] sparse: moves=60 slots=24000 invalidated=3016 retained=51584 retained_fraction=0.945
  frames=84 rays=65832 work=89889 max_frames_per_move=2 intermediates=6
  whole_frames=915 whole_rays=901001 whole_work=2297504 ray_ratio=0.073 work_ratio=0.039
[retention] dense tracking: tracked=1684 untracked=0 outside_cells=5227 checked=4000 bytes=945416
[retention] dense: moves=60 slots=24000 invalidated=3074 retained=98026 retained_fraction=0.970
  frames=108 rays=93013 work=102211 max_frames_per_move=2 intermediates=6
  whole_frames=1952 whole_rays=1942940 whole_work=3007651 ray_ratio=0.048 work_ratio=0.034
```

Single-edit counts on the complete sparse/dense fixtures (move, material edit,
removal, insertion), from the same test:

```
sparse move:      changed_cells=2 invalidated=65 retained=845 dirty=131
sparse material:  changed_cells=1 invalidated=47 retained=863 dirty=83
sparse removal:   changed_cells=1 invalidated=49 retained=861 dirty=85
sparse insertion: changed_cells=1 invalidated=32 retained=872 dirty=74
dense  move:      changed_cells=2 invalidated=54 retained=1630 dirty=117
dense  material:  changed_cells=1 invalidated=42 retained=1644 dirty=78
dense  removal:   changed_cells=1 invalidated=39 retained=1647 dirty=75
dense  insertion: changed_cells=1 invalidated=34 retained=1646 dirty=76
```

Wetland consumer (real `WetlandLighting` state machine, `FakeSink` re-checking
every renderer acceptance condition, from
`a_body_cell_move_retains_the_untouched_gi_faces`):

```
[wetland retention] cells=19 retained=39 invalidated=19 dirty=0 pending=0 dependency_kib=124 gi_live=true
```

The reconciled 60-move volumes are compared bit-for-bit against fresh references
at frames 10/20/30/40/50/60, and the final volume additionally checked through
the app's own publication path. Ray/work/frame counts are engine work units from
the published budget; they are **not** time, and no latency claim is made.

### Delivery snapshot

- Base (frozen 54 head): `0e37e8403bc7ae850eb344ce1d61252ffb325fb5`, which is
  now in `main` (54 merged as `1bdfd6f`); this branch is rebased onto
  `main` @ `48c851e` so the PR carries only these changes.
- Branch: `engine/indirect-dependency-retention`, PR base `main`.
- Files: `crates/matterweave-render/src/indirect/dependency.rs` (new, 1 816
  lines incl. 13 tests), `crates/matterweave-render/src/indirect.rs`
  (+358/−61), `apps/explorer/src/wetland_lighting.rs` (+234/−61), this log.
- Frozen source SHA: `f77a36b1e9c39cc8a2b1d3aa4b1c0dbd5b6bcb5f`
  (`feat(render): retain mesh-proxy indirect faces by exact dependency`), the
  commit carrying `indirect/dependency.rs`, the `indirect.rs` changes and the
  wetland integration on the rebased branch; this log is the commit after it.
  PR: `engine/indirect-dependency-retention` -> `main`.

### Proposed shared updates (log only, not applied here)

- `docs/STATUS.md`: add "exact bounded dependency retention for mesh-proxy edits
  implemented and host-verified; Android visual functional gate still open;
  continuous moving-body GI continuity remains an open requirement" to the
  proxy-lighting entry.
- `docs/performance/README.md`: add an "Engine techniques" entry — link text
  `logs/indirect-dependency-retention.md`, exact proxy-cell dependency retention
  for indirect face values; host counters only, no device claim. (There is no
  `logs/README.md`; `docs/performance/README.md` is the index that lists every
  log.)
- Board/roadmap: split the moving-lighting item into (a) retention (this slice,
  host-verified) and (b) the Android visual gate for motion, which needs the
  phone owner.

### Independent review brief (Codex dispatch)

> Review `engine/indirect-dependency-retention` at the frozen SHA, stacked on 54
> (`0e37e84`). Verify against source, not prose:
> (1) exactness — the recorded dependency is the DDA's read set (empty traversed
> cells and the first hit included) for both traced segments, clipped by
> `segment_exit`; the exposure rule; `UNTRACKED`; the coverage-change fallback;
> (2) boundedness — the index space is the volume box, per-face bitset ≤ 512 B,
> cap accounting, `UNTRACKED` degradation, no unbounded per-ray lists;
> (3) publication safety — complete-only upload, `valid_for`/`source_valid`
> unchanged, invalidated slots zeroed, no stale or partial upload, the
> same-frame republish delta and its app test;
> (4) compatibility — `set_mesh_proxy`, world/sun/epoch clearing, R = 24 /
> SAMPLES = 16 / budgets 1024-8192 unchanged, no shader or `lib.rs` change;
> (5) honesty — the measured counters, the fixture sizes, the app fixture's
> small retention ratio, and the absence of any device or wall-clock claim.
> Expected: ACCEPT or a concrete falsifying fixture.

| Criterion | Verification method | Owner | Result |
| --- | --- | --- | --- |
| Exact bounded retention | Every consulted proxy cell recorded (exposure cells, both segments, empty cells and first hit) or conservative full-clear; explicit cap and failure behavior | this worker | **PASS (host)** — `dependency.rs` recorder + 13 tests; cap clamped to 16 MiB, per-face `UNTRACKED` beyond, `tracked/untracked/resident` reported; 555 KB (sparse, 910 faces) / 945 KB (dense, 1 684) |
| Compatibility | Complete-current-only publication, old API/full clear, R 24 + SAMPLES 16 + budgets 1024/8192 unchanged | this worker | **PASS** — `set_mesh_proxy`/`complete`/`valid_for`/`source_valid` unchanged and tested; constants untouched (diff-limited); no shader/`lib.rs` edit |
| Correctness | Fresh-reference bit-exactness, far occluder, exposure/material/pending-edit/key/failure regressions | this worker | **PASS (host)** — all 24 000 slots compared on move/material/remove/insert/enclose/overlap/coverage/cap; probe-57 counterexample; reversed-sun control; world/sun/epoch clears |
| Real progress characterized | Continuous-motion fixture counts actual retention and convergence, no invented gain | this worker | **PASS (host counts)** — 0.945/0.970 retained, ray ratio 0.073/0.048, work ratio 0.039/0.034, ≤ 2 frames/move, exact references at 7 checkpoints; no wall-clock claim |
| Integrated delivery | Consumer + API together, focused checks, log, commit/push/PR, frozen SHA | this worker | **PASS** — wetland opts in at 6 MiB and reports retained/invalidated/dirty; 122 + 230 tests, clippy/fmt/docs clean; PR based on `main` (`48c851e`), frozen SHA `f77a36b` |
| Review/Android | Independent Opus review, then phone-owner visual functional gate | Codex dispatch | **NOT RUN** — brief above; device gate blocked on the phone worker |

## Insights

- **Exactness is affordable at the scale that matters.** The dependency set per
  face is the DDA read set of 32 short segments, and after the arena fix the
  tracker costs 555–945 KB on the wetland-shaped fixtures, not the 12 MB a
  worst-case preallocated 4 000-cell bitset array would cost. Lazy allocation
  plus the per-face `UNTRACKED` fallback is what keeps the memory bounded
  without losing exactness where it is used.
- **Eliminating box-sized scans, not only rays, is what makes an edit cheap.**
  Rays alone fell to 5–7 % of the whole-volume path, but `work` only improved
  after the `done` bitmap removed the per-never-sampled-face work unit: 0.64 →
  0.039 (sparse) and 0.049 → 0.034 (dense). Convergence per one-cell move went
  from 15/32 whole-volume slices to ≤ 2.
- **The two-segment hazard is caught by construction, not by a radius.** The
  probe-57 receiver is 5–6 cells from the moved body and outside the rejected
  `ceil(R)` bound; its only dependency on the move is a *sun* segment recorded
  through previously empty cells, and it is exactly the face the tracker
  invalidates. A radius-based rule of any size would have kept a wrong value.
- **Retention does not remove the functional gate, only the starvation.** The
  volume still must finish every invalidated face before it may publish, so a
  body moving faster than the ray budget still turns GI off; retention makes the
  off window short (≤ 2 frames here, and same-frame for the small wetland
  fixture) but not zero. Continuous moving-cell GI continuity stays an open
  functional requirement that needs the Android visual gate, not more host
  counters.
- **Honest limits of these numbers.** They are engine work units on a
  20×10×20-cell host fixture with an empty authoritative world and all geometry
  in the proxy. They say nothing about phone frames, thermals or appearance;
  the wetland app fixture itself is small (19 cells, 58 completed faces) and its
  one-cell move retains 39 faces against 19 invalidated — a much weaker ratio
  than the sparse/dense fixtures, which is exactly why the app test asserts
  *behavior* (bounded dirty set, same-frame convergence) rather than a
  percentage.
- **The next necessary measurement is still on device.** If the phone gate
  exists, the useful follow-up is a fixed-camera scripted body path with the
  per-frame `retained`/`invalidated`/`dirty`/`pending` report line, compared
  against per-frame fully converged references. If the valid fraction near a
  moving body in the real wetland scene is low, the next change is a smaller
  budget-filling unit (fewer faces per key), not a weaker dependency rule.
