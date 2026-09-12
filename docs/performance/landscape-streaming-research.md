# Distant terrain research memo (StageS2)

Scope: decision input for a bounded distant-terrain prototype. Base `7dace54`. No code,
build, device or app change in this research. Stage S2 is defined by the concurrent autonomous microvoxel roadmap; its document
was not yet on this frozen code base during the worker run.

## External source status

Primary source: Reddit thread `1vrqmul` "improving render distance in my micro voxel
engine", https://www.reddit.com/r/GraphicsProgramming/comments/1vrqmul/improving_render_distance_in_my_micro_voxel_engine/.
Verification attempts: `www.reddit.com` returned HTTP 200 but an 8.3 KB JavaScript shell with no post body; `old.reddit.com` returned HTTP 302; `.json` returned HTTP 403. The
thread content could not be read. **The creator's description (mesh macrochunks at lower
resolution, deterministic props and edits retained at multiple LODs) is therefore
labelled supplied/unverified by this worker. The lead independently retrieved the
creator post through the web tool during reference research; it supports macrochunks,
deterministic prop IDs and retained edits, not any claim about our code.** No video was watched.

## Current capability vs gap (read evidence)

- Authoritative data: sparse 16³ byte-material chunks, Euclidean chunk addressing,
  bounded edits and per-chunk revisions in `crates/matterweave-core/src/lib.rs:33-195`.
- Legacy core streaming (distinct from the production wetland detail scene): one resolution, 7×7×3 window at radius 3, world limit ±256 cells, Y −16..32,
  ≤512 overrides, synchronous window swap in `crates/matterweave-core/src/streaming.rs:5-8,116-190`.
- Async bounds: one pending stream job, 32 mesh jobs, 8 results, 8 MiB results in
  `crates/matterweave-core/src/async_world.rs:21-29,165-255`.
- Renderer: chunk map keyed `[i32; 3]` with `upload_chunk`/`retain_chunks`/`chunk_revision`
  in `crates/matterweave-render/src/lib.rs:1094,1351-1384`; frustum culling in
  `crates/matterweave-render/src/frustum.rs:1-40`; camera far plane 240 m in
  `apps/explorer/src/controls.rs:196-200`.
- Existing multiresolution: instance/prototype LOD (Source/Half/Quarter, hysteresis,
  dilation bias, budgeted mesh cache) in `crates/matterweave-detail/src/scene.rs:690-717,915`
  and `crates/matterweave-detail/src/select.rs`.
- Gap in this legacy core path: terrain has one resolution and one ring; residency ends at the stream
  radius and the ±256 limit, not at the 240 m far plane. `generated_chunk`
  (`streaming.rs:192-238`) generates full-resolution heightfield only; props are
  detail-crate placements, not core terrain.

## Reuse comparison

1. Reuse `DetailScene` LOD selection for terrain: rejected as the narrow slice. It is
   instance/prototype-oriented (string IDs, metre bounds, quarter-turn placements) and
   living in the D1-owned crate; force-fitting `World` chunks duplicates authority and
   crosses session ownership.
2. Extend core streaming with a coarse derived level: recommended. Reuses deterministic
   generation, override/revision bookkeeping, the bounded async queue pattern and the
   renderer upload path. Cost is level-aware chunk identity, which the renderer and
   override map do not have today.
3. Downsample canonical source chunks: valid deterministic alternative if all inputs
   come from the same source revision. Resident chunks alone are insufficient for the
   distant region. Compare bounded generation/downsampling with direct coarse
   generation; do not treat missing resident data as mathematical nondeterminism.

## Recommended next slice (one bounded prototype)

Add `crates/matterweave-core/src/coarse.rs`, a pure derivation from authoritative source:

- Input: `seed: u64`, `level: u8` (1+; edge `16 << level`), `key: [i32; 3]`, plus the
  world's override map. Output: `Option<Chunk>` with a revision covering overlapping
  source cells.
- Occupancy rule: a coarse cell is solid iff any covered source cell is solid
  (conservative superset → no holes); material by deterministic priority. No prop
  placement in this level; props stay with the existing stable-ID placements.
- Keys use `div_euclid`/`rem_euclid` so negative coordinates match positive boundaries.
- Slice 1 changes no `World` fields, renderer, app or manifests. Inline tests in the new module only, with public-module wiring coordinated by the
  lead. D2 owns all `crates/matterweave-core/tests/**`, including new files. Reserve
  the new module explicitly before dispatch; it is a proposal, not live ownership.
- Deferred slice 2 (needs coordination): level-aware residency key and renderer
  `ChunkKey { level, cell }`, sharing the existing queue/result bounds.

Discriminating tests (Gate A, correctness):

- Determinism: same `(seed, level, key)` yields identical voxels across generation order and reloads; verify negative
  coordinate partitioning without assuming the seeded terrain is mirror-symmetric.
- Conservativeness: every solid source cell projects into a solid coarse cell; a
  deep one-cell opening may be filled (documented, fine ring still authoritative).
- Edit propagation: an edit inside the fine ring advances the revision of every
  overlapping coarse chunk and regeneration matches the declared aggregation rule; use an edit that changes
  aggregate occupancy to test visible change. An untouched chunk keeps its revision.
- Integration gate for slice 2: saturating the queue drops/re-requests coarse work without growth; a stale
  coarse result is rejected after a source edit or level change.

Gate B (visual, discriminating): fixed camera and quality settings compare fine-only vs coarse-beyond-radius over the same world: no unexplained hole at the transition and no coarse claim about frame time. Reuse the existing fixture thresholds and policy in
`docs/performance/renderer-comparison.md`; this is not ray/mesh/hybrid selection evidence
and does not close ADR-0006.

Adoption gate: adopt only if Gate A passes and Gate B shows no hole/misalignment, with
coarse never queried by physics (`stream_contains_position`/colliders stay fine). Report
mesh bytes and preparation counts as bounded budget, not a mobile or kilometre guarantee.
If Gate B fails, keep the coarse level research-only.

## Constraints and open risks

- Persisted edits must affect coarse regeneration after fine chunks are evicted and
  reloaded. Invalidation cannot be limited to the currently resident fine ring.
- Bounded queue/residency: coarse requests must share or explicitly cap against
  `MAX_QUEUED_MESH_JOBS`, `MAX_MESH_RESULTS`, `MAX_MESH_RESULT_BYTES` (8 MiB).
- f32/large world: `stream_center_of` floors f32 to i32 and clamps to ±16 chunks; at
  4096 m positive f32 spacing is approximately 0.488 mm (2^-11 m), not 0.5 m.
  Assess precision against local cell size; camera-relative rendering and integer
  world origins are candidates if precision tests expose a gap. No kilometre guarantee is claimed here.
- Seam fallback: conservative coarse occupancy plus fine-ring priority is the fallback;
  no blending or temporal stability claim is made.
- Dependencies: slice 1 is a pure derivation prototype against existing core APIs and does not touch D1
  detail, D2 physics/core-test files or D4 app/audio. Slice 2 requires coordination with
  those owners before any renderer or app change.

Lead disposition: useful decision input after correcting precision, ownership,
source availability and edit-invalidation claims. No prototype adopted or implemented.

## Additional reference raised by the owner: Virtual Matter

[Atomontage](https://www.atomontage.com/) describes Virtual Matter as progressively
streamed microvoxels with persistent editing, destruction/building, agent-assisted
creation and browser distribution, with optional native apps. These are vendor
claims, not tests performed by this project. [Its public scripting API](https://docs.atomontage.com/api/AE/)
is a useful research entry point. A native client is not evidence of an available,
embeddable Rust/Android engine SDK. SDK access, integration fit, offline behavior,
source/licensing terms and costs remain unverified; no adoption or migration is
selected. The owner clarified: assess transferable technology and concepts, with the entire
runtime on device, and include Lay of the Land. See the
[completed assessment](virtual-matter-and-lay-of-the-land.md). Current disjoint
implementation continues unchanged.
