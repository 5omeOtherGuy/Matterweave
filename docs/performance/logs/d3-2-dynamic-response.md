# Engineering log d3-2-dynamic-response

Worker slice D3.2 (recovery). Branch `engine/recovered-dynamic-lighting`, base
`d2b3e0b`. No push, no PR: integration is the lead's row.

Owned writes used: `crates/matterweave-render/src/indirect.rs`,
`crates/matterweave-render/src/reflection.rs`,
`crates/matterweave-render/src/indirect_tests.rs` (recovered, accepted as-is),
this log. `lighting.rs` was read and left unchanged (no sun/shadow contract
change is needed). `lib.rs`, `world.wgsl`, `shadow.rs`, `ray_reference.rs`,
`static_scene.rs`, `async_indirect.rs`, `apps/**`, `Cargo.lock`, CI,
`docs/STATUS.md` and every other path were not edited.

Run dir for unique verification logs:
`/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/orchestration/lighting-response-muse`
(`d3-2-verify-*.log`; repository diff stays limited to the owned paths above).

## 1. Recovery account

The original worker's `indirect_tests.rs` D3.2 section (six tests, L1/L2/L3/L5/L6/L7)
was salvaged intact and is present in this checkout as the only modified file. Its
`indirect.rs` and `reflection.rs` D3.2 modifications were unreadable (failed-drive
worktree preserved, never accessed); this checkout holds the committed D3.1
versions. The reconstruction below was derived from the recovered tests plus the
stated D3.2 contract, with the tests treated as evidence rather than oracle: every
physics assertion was checked against the implementation before being accepted, and
no test needed correction (section 4).

## 2. What changed, per owned file

### 2.1 `indirect.rs` — completion-gated publication plus a declared work bound

Three additions, no change to the gather physics, the SAT rasterization, world
authority (`scene_material`/`trace_scene` untouched) or the key shape:

- `IndirectVolume::complete()` (`indirect.rs`): true only for a finished key
  (`key.is_some() && cursor == values.len()`). A fresh, cleared, partially
  updated or invalid-light volume is incomplete.
- `IndirectVolume::pending_work()` (`indirect.rs`): upper bound on the remaining
  `work` units for the current key,
  `(values.len() - cursor) * samples - sample_index`, zero when complete. Each
  face slot needs at most `samples` inspections, so a changed key starts with at
  most `values.len() * samples` pending and every progressing `update` slice
  strictly reduces it.
- `valid_for` and `source_valid` now require `complete()`. An incomplete volume
  is never valid for sampling publication and never publishable through
  `upload_indirect` (which checks `source_valid`; `lib.rs` itself is untouched,
  so the tighter gate applies without an entry-point edit).

Why this is the missing piece: the recovered L6/L7 tests call `complete()` and
`pending_work()` (12 compile errors before this change) and assert that a
zero-work `update` observing a key change clears the old radiance in that same
call while neither the old nor the new key validates until recomputed. The prior
`valid_for`/`source_valid` returned true for a freshly keyed but uncomputed
volume, which would have let a superseded key validate.

### 2.2 `reflection.rs` — representation already sufficient; two discriminating tests added

No representation change was needed: `pack_with_mesh`, `source_mesh_digest`,
`valid_for` and `valid_for_scene` already give movement identity, edit
invalidation and supersession refusal for the packed grid, and the D3.1 oracle
(`reflect_sample_with_mesh`) already traces the world-plus-proxy union. The
reconstruction therefore adds only the missing D3.2 coverage, reusing the
existing C7/C8 fixtures (`mesh_world`, `mesh_table`, `mesh_proxy`):

- R1 `moving_reflector_redirects_response_and_static_control_holds`: packs built
  from a rest proxy vs a lifted proxy (plus one shared static control object)
  carry different digests; the c8 world-mirror ray hits the object at rest and
  the far wall when lifted, at a longer distance; a second reflection off the
  same mirror travelling −X (away from both positions and the wall) and the
  control object's packed cells are bit-identical; each pack passes
  `valid_for_scene` only for its own digest.
- R2 `world_edit_updates_reflectivity_and_superseded_pack_is_refused`: removing
  the two wall cells on the mesh-top ray's climb turns a `[3, 3, 0]` wall hit
  into a miss while the mesh cell keeps its material/mirror; the prior pack
  fails both `valid_for` and `valid_for_scene` for the edited scene, including
  after an A→B→A round trip at a new revision; the footprint outside the wall
  and the rebuilt proxy digest are stable while the edited footprint changes.

Two defects in the first draft of R2 were caught by running it, both test bugs
rather than implementation bugs: one wall cell was not enough (the ray climbs
from `[3, 3, 0]` into `[3, 4, 0]`, measured hit before correction), and the
footprint comparison ran after the round-trip restore (digests trivially equal).
Both are fixed in the committed test, which fails on the pre-fix draft and
passes now.

### 2.3 `indirect_tests.rs` — recovered D3.2 section accepted without correction

The six recovered tests (L1 moving sun, L2 enclosure, L3 voxel edit, L5 colour
bleed, L6 latency, L7 stale publication) compile against 2.1 and pass
unmodified. Their controls were verified meaningful rather than assumed: the
divider-wall shadow geometry (slope-0.5 low sun shadowing the full five-cell
down-sun half), the sealed-room exact-zero bound, the away-facing control face,
the hue-swap symmetry and the sliced-vs-whole bit-identity all assert the
directions the contract states.

### 2.4 `lighting.rs` — unchanged

The moving-sun contract is carried by the normalized `light_key` (a scaled sun
direction is the same light; a tilted one is not), which already exists. No sun,
shadow-camera or settings change is required.

## 3. Declared criteria and results (asserted, not eyeballed)

| # | Asserted criterion | Result |
| --- | --- | --- |
| L1 | low sun toward +X lights the +X wall face (`> 0.05`) and zeroes the −X face exactly, reverse sun flips the pair, wall top stays `[0,0,0]` under all three suns, scaled sun shares the light key | PASS (`moving_sun_redirects_indirect_response_and_static_control_holds`) |
| L2 | sealed room leaks at most `SEALED_ENCLOSURE_LEAKAGE_MAX = 0.0` on every interior face; 3×3 skylight admits strictly more but no more than the fully open roof (`> 0.03`); closing the roof clears the cache in the invalidating update and recompletes under the bound | PASS (`enclosure_response_follows_the_opening_and_sealed_leakage_is_bounded`) |
| L3 | one `World::set` (red floor → sunlit green voxel) lowers red and raises green on the facing receiver face while the away-facing control face is bit-identical, the proxy digest is untouched, the untouched footprint digest is stable, the edited one changes, and the updated volume equals a from-scratch computation | PASS (`voxel_edit_updates_indirect_and_leaves_untouched_identity_unchanged`) |
| L5 | the two faces of a column on a half-red/half-green floor report their own half's hue (`> 2×` channel dominance, `> 0.05`), and swapping the albedos swaps the responses | PASS (`colour_bleeding_carries_the_source_hue_to_the_neighbouring_surface`) |
| L6 | fixture bound: 48 face slots × 8 samples = at most 384 pending work units, 32-unit slices complete in at most 12 calls with strictly decreasing remainder, sliced result bit-identical to one full-budget call | PASS (`lighting_latency_is_bounded_and_declared`) |
| L7 | superseding light clears all sampled cells in the same zero-work call, neither key validates while incomplete, completing the new light reproduces a fresh volume bit for bit, and re-requesting the old light clears again with no mixed partial state | PASS (`superseded_lighting_never_becomes_visible`) |
| R1 | lifted reflector redirects the c8 ray from object hit to farther wall hit; −X control reflection and control-object cells bit-identical; per-digest `valid_for_scene` | PASS (`moving_reflector_redirects_response_and_static_control_holds`) |
| R2 | two-cell wall removal turns the mesh-top wall hit into a miss; old pack refused by both checks including after A→B→A; untouched footprint and proxy digest stable | PASS (`world_edit_updates_reflectivity_and_superseded_pack_is_refused`) |
| D3.1 | all pre-existing indirect/reflection contracts preserved | PASS (107 lib tests, section 6) |

## 4. Latency bound: accurate scope (read this before quoting any number)

The implemented bound is a **CPU work-unit bound on the preparation/publication
mechanism**, and nothing else:

- Named bound: `LATENCY_WORK_BUDGET = 32` work units per `update` slice,
  `LATENCY_CALL_BOUND = 12` slices for the 48-slot/8-sample fixture
  (`pending_work()` upper bound 384, asserted in L6). `UpdateStats.work/rays`
  are per-slice face-inspection and DDA-call counts, capped by
  `MAX_UPDATE_WORK`/`MAX_UPDATE_RAYS`.
- Explicitly **not** measured or claimed: Android frame time, presentation
  latency, fence/wait behaviour, GPU upload timing, thermal or resolution-gated
  thresholds. No benchmark suite was run (per dispatch) and no such number
  appears in any test or assertion.
- The unimplemented gate between publication and presentation (frame pacing,
  swapchain hand-off, any device-side timing) is handed to the lead with the
  entry-point proposals in section 5. Do not present L6 as a frame budget.

## 5. Proposed diffs for the lead (not applied; checkout keeps old signatures)

The lead is applying `upload_indirect(..., mesh_digest: Option<u64>)` comparing
the caller's current proxy digest and
`upload_reflection(..., mesh_digest: Option<u64>)` calling `valid_for_scene`,
both retaining rejection when resident GPU mesh geometry has no proxy. This
slice stops at that boundary; the exact interactions the owned code supports:

- For `upload_indirect`: after this slice, `source_valid` already rejects
  incomplete volumes, so the lead's addition is one comparison —
  `volume.mesh_digest() == mesh_digest` — before the existing
  `source_valid(world, source_epoch)` check, plus the no-proxy rejection while
  dynamic/static-scene mesh geometry is resident (mirroring the existing guard).
  Rationale: today a volume whose proxy was never re-attached after a move keeps
  an old digest inside its key; the comparison turns "the caller attached some
  proxy" into "the caller attached the current one".
- For `upload_reflection`: replace the existing
  `volume.valid_for(world, source_epoch)` check with
  `volume.valid_for_scene(world, source_epoch, mesh_digest)`, plus the
  mirror-image no-proxy rejection when `volume.source_mesh_digest().is_none()`
  while mesh-only geometry is resident. Call sites passing the new argument are
  the lead's row (`apps/explorer/src/reflection_check.rs`,
  `crates/matterweave-render/examples/reflection_smoke.rs`).
- No WGSL, descriptor-layout or buffer-size change is proposed in this slice:
  proxy cells and face values already ride the existing bindings 3/4/5 wholesale.

## 6. Verification run (exact commands)

```
export CARGO_BUILD_JOBS=1
export CARGO_TARGET_DIR=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/lighting-response-target
cargo test -p matterweave-render --lib --locked
  -> 107 passed, 0 failed (99 before this slice: 6 recovered L-tests + 2 new R-tests)
cargo clippy -p matterweave-render --all-targets --locked -- -D warnings
  -> exit 0; one pre-existing vendor warning (vendor/winit/.../x11/ime/context.rs,
     function_casts_as_integer), untouched by this slice, not promoted to an error
cargo fmt --all -- --check
  -> clean (after one fmt pass over the new R-tests only)
python3 tools/check_docs.py
  -> PASS
```

Full command outputs are kept as unique logs in the run dir
(`d3-2-verify-*.log`); the repository diff is limited to the owned paths in the
header. No whole-workspace build, no benchmark suite, no device run was executed
from this session; nothing here is a device result, a frame time or a visual claim.

## 7. Limitations carried forward (D3.1 section 5/7, unchanged by this slice)

1. **Sub-cell (fractional) mesh self-reflection is not fixed and is not claimed
   fixed.** The packed grid carries no instance identity and the shipped
   start-cell self-hit rule remains, so a mesh-only surface strictly inside its
   own proxy cell still terminates its own reflection as background/fog
   (pinned by G1, untouched). Grid-aligned mesh faces reflect correctly (C8, R1).
2. **Fragment-normal alignment for indirect reception** (`world.wgsl`
   `indirect_diffuse` returns zero for non-axis-aligned normals) is unchanged;
   this slice changes what is represented, not that lookup rule.
3. **One-cell resolution, conservative rasterization** (SAT triangle/cell test,
   contacts marked, planar faces mark the faced-into cell) is preserved
   byte-for-byte; no rasterization code was touched.
4. **Coverage box is the caller's choice**; geometry outside it does not
   participate. `MeshProxy::build` remains a load/edit-time operation bounded by
   `MAX_MESH_PROXY_TESTS`/`MAX_MESH_PROXY_CELLS`, not a per-frame cost.
5. **Async path** (`async_indirect.rs`) is untouched and stays unit-voxel-only;
   per the D3.1 log it needs the proxy (or its geometry + coverage box) carried
   into `AsyncIndirectConfig` with the digest joined to `SourceKey` before it can
   share this slice's movement semantics.

## 8. Definition of done

| Criterion | Verification | Owner | Result |
| --- | --- | --- | --- |
| Moving sun response plus nondrifting control and hue bleed | L1, L5 test bodies | worker | PASS |
| Enclosure open/close and edits have bounded leakage/invalidation | L2, L3, R2 test bodies | worker | PASS |
| Moving/static reflector behavior and superseded source refusal | R1, R2 test bodies | worker | PASS |
| Latency bound accurately scoped to implemented work/publication mechanism | L6 plus section 4 scoping | worker | PASS |
| Existing D3.1 contracts preserved, source scoped and committed | 107 render lib tests, scoped strict Clippy, fmt, this log | worker | PASS (lead completed the omitted commit at handoff) |
| Entry-point/shader/Android integration | section 5 proposals | lead | NOT RUN |


## Actions Taken

Muse reconstructed the missing completion gate and reflection response tests from
committed D3.1 plus the salvaged indirect tests. The lead inspected the resulting
completion/publication change and preserved the verified diff as a commit.

## Issues & Friction

The previous drive's partial implementation was unreadable. The recovered tests
initially had 12 missing-method compile errors; two newly written reflection test
fixtures required geometry/order corrections. The worker incorrectly treated the
requested commit as a lead-only task and used different log headings. The lead
completed those handoff requirements; this is accepted-after-repair process work,
not a first-pass compliant handoff.

## Decisions & Rationale

Retain the existing gather and proxy rasterization; tighten publication validity to
complete results. A CPU work-unit bound is distinct from device frame latency.

## Solutions Applied

Added completion/pending-work introspection and incomplete-publication rejection,
plus response controls and supersession checks. 107 renderer tests, scoped strict
Clippy, fmt and docs checks pass. Independent final review and Android remain
lead-owned and are not claimed by the worker.

## Insights

A key can match before its cache is computed. Matching provenance alone does not
make a partial cache safe to publish. Recovered tests can reconstruct a small
missing implementation when their behavioral assumptions are verified.
