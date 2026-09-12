# Packed-occupancy hierarchical voxel traversal experiment (issue 42)

Worker log for the bounded M2 traversal experiment. Base `fc6212e0` on
`origin/main`; branch `engine/ray-hierarchy-experiment`; new isolated workspace member
`crates/matterweave-ray-hierarchy`. Nothing in the retained reference, the renderer,
`apps/explorer` or any shared authority file was edited.

**State: functional candidate GREEN on CPU. GPU and Android execution NOT RUN. No
performance claim, no primary-path decision, no DAG, no production replacement.**

## Question and answer

Can a bounded crop of authoritative voxel material be traversed through a compact packed
block-occupancy structure with the same hits, materials, normals and depth as the retained
dense reference, while keeping occupancy separate from materials, bounding build/update
memory, and keeping staleness explicit?

Yes on CPU, with three modes over one shared entry and step implementation:

| Mode | Memory access | Result |
| --- | --- | --- |
| `Reference` | one material word per visited cell | retained lineage, anchors the comparison |
| `BlockMask` | packed block occupancy decides access: an empty block costs one word load and no material reads; a set bit gates each material read | bit-for-bit identical hits to `Reference` |
| `BlockStep` | `BlockMask` plus a coarse step over empty blocks with an exact fine-plane catch-up | bit-for-bit identical hits, 27% fewer outer iterations on the corpus |

The candidate is *not* a proven optimization. The counters below are buffer-access counts
and plane-formula evaluations; no wall-clock or device measurement exists.

## Existing-solution assessment

Inspected before writing code, in the order the component policy requires.

| Candidate | Provenance verified this session | Assessment | Decision |
| --- | --- | --- | --- |
| Retained reference lineage: `matterweave-render/src/ray_reference.wgsl` + `World::raycast` | in repository at `fc6212e0` | bounded crop, slab entry rules, tie stepping, lowest-axis normals, iteration cap; CPU oracle in f64 with accumulated plane distances | **Reused as the reference.** Every mode shares this crate's transcription of its entry and step rules; the CPU oracle anchors the transcription |
| `matterweave_render::ray_reference::RayVolume` | in repository at `fc6212e0` | bounded crop packing, `MAX_CELLS`/`MAX_AXIS`/`+/-8192` validation, epoch/revision/seed staleness | **Reused.** `HierarchyVolume::from_reference` takes a pack; crop acceptance is asserted equal to `RayVolume::pack` for a matrix of requests |
| DeadlockCode `voxel_ray_traversal` @ `a91feb3b57daf09139f64a15a9cb1a902ed0ea26`, `shaders/traverse.comp` | `gh api repos/DeadlockCode/voxel_ray_traversal` → dual MIT/Apache-2.0 (246 stars, pushed 2025-09-06); file fetched from the pinned raw URL | per-voxel 3D DDA with a 4x4x8 packed occupancy texel (`rgba32ui`, 128 bits), bit test per cell, last-texel reuse in `readVoxel`, no tree/BVH, no material lookup separation | **Adopted at algorithm level only.** The block packing, bit-gated read and cached-block reuse are reimplemented from the published idea; **no source was copied**, so no license obligation, `NOTICE` or attribution burden is imported. Its word layout (x inside the low bits of a `uvec4` texel) was not copied either; this crate pins a linear `x + sx*(y + sy*z)` bit order so tests are exact |
| Cubiquity traversal (named in issue 42, no URL) | the historical host is not retrievable: `api.bitbucket.org/2.0/repositories/volumesoffun/cubiquity` returns "workspace ... deactivated due to inactivity"; `bitbucket.org/volumesoffun/cubiquity` returns HTTP 404; no GitHub mirror under `volumesoffun/*` | the closest verifiable upstream is a third-party PolyVox clone (MIT), which was not inspected line by line in this slice | **Rejected for provenance, deferred on merit.** Adopting an unverifiable source or a second-hand copy would fail the source/license record requirement; a later session with a reachable source can reopen it |
| Existing `matterweave_core::CoarseTile` hierarchy (levels 1-2, 16^3 tiles) | in repository at `fc6212e0`, `crates/matterweave-core/src/coarse.rs` | conservative any-solid aggregation into byte-material tiles derived through `World::coarse_tile`, with whole-world revision provenance and streaming-domain rules; `material()` is a lossy topmost summary | **Not adopted for the traversal.** Its aggregation is safe for skipping, but its unit is a 16^3 byte-material tile at 2/4-cell granularity, not a 4x4x8 bit block with per-cell row reuse, and its material field must not answer hits. Recorded rather than duplicated: a later session could feed conservative tile occupancy into this crate's grid |
| Custom packed-occupancy traversal | this session | needed because the reference reads one material word per visited cell and no existing bounded representation in the repository packs per-cell occupancy blocks for traversal | **Built**, with the comparison above as the evidence |

Runtime dependencies added: none. `matterweave-core` and `matterweave-render` are path
dependencies; `glam` was already pinned for the render crate and is a dev-dependency here.
`Cargo.lock` gains only the new workspace member.

## Reference semantics pinned by this crate

Inherited and shared by every mode through one entry routine and one fine-step routine:

- half-open crop `[origin, origin + dimensions)`, `MAX_CELLS`, `MAX_AXIS` and `+/-8192`
  bounds from `RayVolume::pack`;
- slab clipping that branches on zero direction components instead of dividing; a parallel
  ray on the upper face of a crop axis is outside, on the lower face inside;
- zero-length AABB overlap rejected before any cell is inspected;
- external entries snap only known slab planes, then cross all exactly tied internal grid
  planes together; an origin already inside inspects `floor(origin)` first;
- tied axes step simultaneously, the lowest crossed axis supplies the normal, and a
  tie-crossing origin reports a zero normal with distance 0;
- distances recomputed from integer planes every step (`f64` here, the host analogue of
  the shader's f32 recomputation), never accumulated;
- outer iteration cap `dims.x + dims.y + dims.z + 1`, asserted not to be exhausted;
- negative coordinates index by floor, not truncation.

Deliberate deviations, each asserted in tests:

1. Direction handling. The shader derives a direction by normalizing an unprojection
   segment; this crate takes a unit direction plus a clipped segment length, so both
   comparison modes use identical geometry. Nonfinite, zero and non-unit directions are
   rejected with a typed error instead of returning a miss.
2. `World::raycast` accumulates `next += delta` in f64 from the world origin over the whole
   world; this crate recomputes crossings and excludes everything outside the crop. Two
   differences follow and are asserted explicitly rather than smoothed over:
   **crop exclusion** (out-of-crop content is invisible, including a foreground occluder
   and an out-of-crop solid at the ray origin) and the **zero-length overlap rule** (an
   exact lower-face origin moving outward misses here while the CPU DDA reports a t0 hit).
3. Depth is not written by a fragment shader here. `clip_depth` mirrors the reference's
   `clip.z / clip.w` from the reported hit distance; the projection matrix remains a caller
   precondition.
4. Counters exist that the shader does not have; they are structural, not temporal.

Unsupported or unverified cases, stated plainly:

- GPU/Android execution, mobile precision and driver behavior: **NOT RUN**.
- Primary-path selection, quality comparisons against rasterization and costs at
  representative quality: out of scope here; ADR-0006 stays Proposed.
- Multi-level trees, DAGs, SVDAG compression, edits at fine-detail prototypes
  (`matterweave-detail`), moving objects and streaming residency: not modelled. The crop
  is a fixed bounded volume, not a world-scale delivery.
- Exact grid-plane ties where the accumulated (oracle) and recomputed (reference) f64
  crossing distances round differently. Three cases in 765 seeded comparisons resolved a
  cell on the other side of one plane at the same distance; the test classifies exactly
  that shape of disagreement and fails on anything else. This is the same class of
  difference the retained reference documents between f32 WGSL and f64 CPU.

## Implementation and bounded contracts

`HierarchyVolume::from_reference(&RayVolume, BlockShape)` derives an immutable-in-world
snapshot: origin, dimensions, a copy of the dense `u32` material words, a packed occupancy
grid and a `SourceKey`. `build` accepts raw crops under the same rules and rejects
materials above `u8::MAX`, because hits report `matterweave_core::RayHit`.

- Occupancy: one bit per cell in fixed blocks (`BlockShape`, at most 512 bits), stored in
  `u32` words in the same order as the dense index, `bit = x + sx*(y + sy*z)`. Empty blocks
  are all-zero words; a partial edge block exists and holds set bits only for in-crop
  cells. Bits are derived from the material words and never replace them.
- Bounded build: one scan of the material words, `blocks * words_per_block` occupancy
  words, both allocated fallibly (`try_reserve_exact`); `BuildReport` returns the exact
  counts actually incurred, including `occupancy_bits_set`, which tests assert equals the
  solid cell count.
- Bounded update: `patch_cell(cell, material)` writes at most one material word and one
  occupancy word and reports whether the block became empty or occupied. It never touches
  `World`, sets `locally_edited`, and the snapshot then fails `valid_for` even when the
  source pack is still valid. Rebuilding from an edited world reproduces the patched hits.
- Residency accounting: `MemoryStats` reports material bytes, occupancy words and bytes,
  and the total. For the tested crops occupancy adds one bit per cell, 3.1% of the 4
  bytes/cell material payload (1728 vs 55296 bytes for a 24^3 crop); it does not reduce the
  dense material requirement at all.
- Staleness: `valid_for(&World, epoch)` requires matching epoch, whole-world revision and
  seed, identical to `RayVolume::valid_for`, which a test compares row by row; a locally
  patched snapshot is stricter by design.
- Traversal work bound: `max_iterations()` is the reference's
  `dims.x + dims.y + dims.z + 1`. Every outer iteration of every mode crosses at least one
  fine plane, so coarse steps fit the same cap; tests assert the cap is never exhausted and
  that `iterations == fine_steps + block_steps` or one more.
- Coarse step: the empty block's exit plane is crossed and each axis is caught up through
  the fine planes strictly before it with the same plane formula, so the cell state matches
  the reference's per-cell walk exactly; a plane landing exactly on the exit jumps to the
  new block's entry cell, while a colliding interior plane steps one cell and can supply
  the normal. The catch-up cost is counted, not assumed away.

## What was verified

Environment: recovery SSD, `RUSTC_WRAPPER=` (no inherited sccache),
`CARGO_BUILD_JOBS=1`,
`CARGO_TARGET_DIR=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/ray-experiment-target`.
`/mnt/bench` was not used. No release build, no workspace-wide rebuild; only this crate's
focused checks were run. 19 GB free before and after, target directory 736 MB.

```sh
cargo test -p matterweave-ray-hierarchy                     # 29 passed, 0 failed
cargo clippy -p matterweave-ray-hierarchy --all-targets -- -D warnings   # clean
cargo fmt -p matterweave-ray-hierarchy -- --check           # clean
cargo test -p matterweave-render --lib ray_reference        # 7 passed (retained reference intact)
python3 tools/check_docs.py
```

Test inventory (29): occupancy layout and bit-word boundaries; material/occupancy
separation; partial edge blocks; invalid crops and material ids; bounded patch writes and
block transitions; random patched cells keeping bits aligned; face entry; zero-length
overlap disclosure; half-open slab rules for zero components; tie stepping with
lowest-axis normals; inside-solid start; negative-coordinate floor; thin wall with a
one-cell opening; ray-input rejection; iteration cap; material-versus-occupancy traffic;
cached block reuse; coarse-step trade-off; block-shape scaling; targeted oracle cases;
seeded oracle corpus; seeded mode corpus; block-boundary corpus; patched-versus-rebuilt;
staleness parity; crop exclusion; depth; snapshot crop acceptance.

Measured differential results, printed by the tests:

| Corpus | Comparisons | Result |
| --- | --- | --- |
| Targeted cases against `World::raycast`, `CUBE4` and `4x4x8`, three modes | 96 hits, 36 misses | cell, material and normal exact; **max distance delta 0**; 0 unresolved disagreements |
| Block-boundary cases against `World::raycast`, four shapes | 20 hits, 36 misses | exact; max distance delta 0 |
| Seeded fixtures against `World::raycast`, `4x4x8` | 259 hits, 506 misses | exact except 3 tied-plane resolutions, each verified as the same distance in face-adjacent cells |
| Seeded fixtures, `Reference` vs `BlockMask` vs `BlockStep`, three shapes, four start classes including origins outside the crop | 743 hits, 1561 misses in 2304 rays | **bit-for-bit equal**: cell, material, normal and `f32` distance bits |
| Depth from the same hits, perspective and orthographic | 36 hits | equal across modes, matches glam's independent matrix product, inside legal Vulkan depth |
| Patched snapshot vs rebuilt snapshot vs edited world | 3 rays x 2 edits | identical hits; the source world stays authoritative |

Structural counters over the 2304-ray corpus (buffer accesses, not timings):

| Mode | iterations | fine steps | block steps | catch-up planes | blocks checked | block word loads | cells examined | material reads |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `Reference` | 11581 | 9649 | 0 | 0 | 0 | 0 | 10392 | 10392 |
| `BlockMask` | 11581 | 9649 | 0 | 0 | 10392 | 3486 | 6021 | 743 |
| `BlockStep` | 8411 | 5278 | 1201 | 3101 | 7435 | 3486 | 6021 | 743 |

Reading of these counts:

- `material reads == hits` for both occupancy modes (743 of 743, asserted): a bit is set
  exactly when a solid cell is present, so the packed occupancy never costs a wasted
  material word and never hides one. `Reference` read 10392 words to reach the same 743
  hits.
- `BlockMask` steps per cell exactly like `Reference` (`iterations` equal) and removes the
  material traffic; an empty block costs one occupancy word load, and the cached block word
  is reused across the cells it covers (`occupancy_word_loads` 3486 < `blocks_checked`
  10392).
- `BlockStep` replaces 3170 fine iterations with 1201 coarse steps and pays 3101 catch-up
  plane evaluations. Counting one crossing-formula evaluation per axis visit gives
  `3 * 11581 = 34743` for the reference against `3 * 8411 + 3101 = 28334` for the coarse
  mode, about 18% fewer evaluations. That is derived arithmetic over counted quantities,
  **not a timing**: no clock, no GPU, no device, no representative quality.
- Occupancy bytes are comparable, not automatically smaller: for a 24^3 crop all three
  shapes need 1728 bytes (one bit per cell); for shapes whose extent does not divide the
  crop, whole-word rounding makes coarser blocks larger (13x16x20 crop: 640 bytes for
  4x4x4, 768 for 4x4x8 and 8x8x8).

## Delivery

Code and tests: `crates/matterweave-ray-hierarchy`; workspace member line in the root
`Cargo.toml`; `Cargo.lock` gains only that member. Review checkpoint (code and tests,
before this log): `057d65dc92ecd8c5f89ccb1662cf5994d37fd2bf`. The frozen reviewed source is the branch head at
handoff; the PR records the exact SHA.

One-line change outside the new crate:

```diff
-members = ["crates/matterweave-core", ..., "crates/matterweave-pacing", "apps/explorer"]
+members = ["crates/matterweave-core", ..., "crates/matterweave-pacing", "crates/matterweave-ray-hierarchy", "apps/explorer"]
```

No renderer file was touched. A GPU port would need a new module and shader registration in
`matterweave-render/src/lib.rs` and `build.rs`, which this slice deliberately does not
apply because `lib.rs` is owned by the concurrent integration worker. Proposed handoff:

```diff
 // crates/matterweave-render/src/lib.rs (NOT APPLIED)
 pub mod ray_reference;
+pub mod ray_hierarchy_gpu;      // owns its own SPIR-V constants and pipeline contract

 // crates/matterweave-render/build.rs (NOT APPLIED)
     for name in [
         "world",
         "hud",
         "shadow",
         "ray_reference",
         "comparison_raster",
+        "ray_hierarchy",        // fragment entry fs_main, same naga 24.0.0 loop
     ] {
```

Device step for that port, in order: reuse `RayUniform` (192 bytes) and add a storage
binding for the packed occupancy words plus the existing material and palette bindings;
dispatch the same fullscreen fragment and compare readback against
`World::raycast` with the crate's targeted cases (tied axes, zero components, negative
crops, inside-solid starts, the thin wall with a one-cell opening) and the seeded corpus;
run `VK_LAYER_KHRONOS_validation` with synchronization validation, as the retained
reference requires; then measure on the phone with the representative comparison issue.
The f32 WGSL port must keep the stateless per-plane formula and the tie rules; f64 host
agreement established here does not transfer to mobile precision, and the retained
reference already needed explicit tolerances on Adreno.

## Decision

- **Adapt** the block-mask level (`BlockMask`) as the traversal candidate for later
  device-cost comparison. It is exactly equivalent to the reference on the CPU corpora,
  needs 3.1% extra payload, and removes the per-cell material traffic.
- **Defer** the coarse-step level (`BlockStep`) and any production replacement: it is
  exactly equivalent and cheaper in counted iterations, but it adds catch-up complexity and
  the device comparison (representative quality, edits, thermal behavior) has not run.
- **Reject** adopting the dead-source Cubiquity traversal for provenance reasons.
- Issue 42's Android functional criterion, the representative total-cost comparison and
  the ADR-0006 selection remain open and are not implied by these results.

## Proposed updates for the integration worker (NOT APPLIED)

`docs/STATUS.md`, new bullet in the current performance/experiment section:

```text
- Bounded hierarchical traversal experiment (issue 42): new workspace member
  crates/matterweave-ray-hierarchy with packed 4x4x4 / 4x4x8 / 8x8x8 block occupancy
  beside dense materials. CPU differential evidence: 2600+ oracle comparisons with no
  measured distance deviation, bit-for-bit equality between the reference, block-mask and
  coarse-step modes over 2304 seeded rays, bounded build/patch accounting and explicit
  staleness. GPU and Android execution NOT RUN; no performance claim; ADR-0006 unchanged.
  Log: docs/performance/logs/ray-hierarchy-experiment.md.
```

`docs/ROADMAP.md`, M2 traversal line: note the experiment as a reviewed CPU candidate
awaiting device functional execution and the representative cost comparison; keep the
DAG/tree decision open.

`docs/performance/board.json`, new task entry:

```json
"RAY-HIERARCHY": {
  "attempt": 1,
  "owner": "ray-hierarchy-experiment",
  "state": "candidate-review",
  "artifact": "crates/matterweave-ray-hierarchy",
  "log": "docs/performance/logs/ray-hierarchy-experiment.md"
}
```

## Friction, decisions and limits

- The instruction to avoid the renderer's `lib.rs` (owned by concurrent integration work)
  made a GPU kernel impossible to register in this slice; the CPU prototype was chosen
  instead, as allowed, and its counters are explicitly not a performance result. Shipping
  an unexecuted WGSL file was rejected as an unverified artifact; the device owner ports a
  tested algorithm against the pinned semantics list above.
- Assessment friction: the Cubiquity workspace returns HTTP 404 and Bitbucket reports it
  deactivated, so that reference could only be recorded as unverifiable. DeadlockCode's
  license is dual MIT/Apache-2.0; nothing was copied, so no obligation was imported.
- Design decision worth recording: a coarse step cannot be a naive jump to the block exit.
  A ray changes cells inside an empty block along the axes it is not exiting through, so the
  cell state must be caught up through those fine planes before continuing, and exact ties
  must keep the reference's simultaneous-stepping rule. The first draft of the coarse step
  in this session only updated the exit axes; the differential corpus caught it as a
  cell-sequence divergence. The catch-up and tie handling in `skip_empty_block` are the fix,
  and the corpus plus the block-boundary cases guard it.
- Limit: crop-excluded content is invisible by construction, so comparison fixtures and the
  future device scenes must fit inside the box; the whole-world oracle cannot be used
  directly for a cropped answer.
- Limit: the crate models one block level over a fixed crop. It does not answer how the
  structure would be built or updated incrementally for a streaming world, and its
  per-edit patch rewrite (one material word plus one occupancy word) assumes a resident
  crop rather than a GPU upload schedule.
- Not measured: wall-clock time on any platform, GPU residency, upload cost, thermal
  behavior, mobile precision. The counters here must not be quoted as performance.
