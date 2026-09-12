# Engineering log w_547ae261

Worker: `deepseek-flash-go` · outcome: Category: explore · run: `/mnt/bench/matterweave-dev/microvoxel-roadmap/research`

- **Actions Taken:** Read `docs/adr/0005-voxel-world-data.md`, `docs/adr/0007-virtualized-detail.md`,
  `crates/matterweave-core/src/{lib.rs,streaming.rs,async_world.rs,mesh.rs}` API surface,
  `crates/matterweave-render/src/{lib.rs,frustum.rs}`, `crates/matterweave-detail/src/{scene.rs,select.rs}`
  LOD API, `apps/explorer/src/{lib.rs,controls.rs}` streaming/render wiring, and
  `docs/performance/renderer-comparison.md`. Attempted external primary source three ways
  (`www.reddit.com` HTML shell 8.3 KB, `old.reddit.com` HTTP 302, `.json` HTTP 403) to verify the
  creator's macrochunk description. Wrote `docs/performance/landscape-streaming-research.md`
  (100 lines) and this log. Did not build, run tests, use a device, or commit/push.
- **Issues & Friction:** The primary Reddit source is not machine-readable from this environment
  (JS shell / redirect / 403), so the creator summary is labelled supplied/unverified in the memo.
  "StageS2" is not a repository term (`grep -rn StageS2 docs/` empty); treated as the brief's
  mission label. The renderer chunk map is keyed `[i32; 3]` with no level component, so a coarse
  terrain level needs new identity plumbing before any integration.
- **Decisions & Rationale:** Recommended extending core streaming with a deterministic coarse
  derivation (`crates/matterweave-core/src/coarse.rs`) over reusing `DetailScene` LOD selection,
  because the detail path is instance/prototype-oriented and D1-owned, while core reuse keeps
  ADR-0005 source authority and the existing override/revision/bounded-queue machinery. Rejected
  downsampling resident fine chunks because only a 7×7 fine ring exists, making coarse output
  camera-history dependent. Recommended conservative any-solid occupancy so the coarse level is a
  superset (seam fallback = fine ring priority, no holes claimed as smooth blending).
- **Solutions Applied:** Defined slice 1 as a pure function over `(seed, level, key, overrides)`
  with no `World` field, renderer, app or manifest change and new tests only in
  `crates/matterweave-core/tests/coarse.rs`; deferred level-aware residency/renderer keying to a
  coordinated slice 2. Wrote an explicit adoption gate: Gate A determinism/conservativeness/edit
  propagation/bounded-queue tests, Gate B fixed-camera no-hole comparison reusing the existing
  fixture thresholds without claiming ray/mesh/hybrid selection. Stated f32/i32 limits and no
  kilometre or performance guarantee.
- **Insights:** The real distant-terrain gap is not mesh generation but identity and residency:
  edits, overrides, revisions and renderer uploads are all keyed to one 16³ level. A coarse level
  that derives from authoritative source (not resident fine chunks) is the only variant compatible
  with negative coordinates, deterministic regeneration and ADR-0005/0007's still-open gates.

## Result

| Criterion | Verification method | Owner | Result |
| --- | --- | --- | --- |
| Source-backed current gap and reuse alternatives | Named paths and read evidence | worker | PASS |
| One bounded next implementation with testable adoption gate | Explicit input/output, tests and exclusion of other assignments | worker | PASS |
| Documentation valid | `python3 tools/check_docs.py` | worker | PASS (185 files, 581 links, 16 ADRs, 20 requirements) |
| Final acceptance | Review memo against inspected sources | lead | NOT RUN |

Scope exclusions honoured: no D1 detail, D2 physics/core-test, or D4 audio/app file changes; no
edits to STATUS, ROADMAP, board, CI or Cargo manifests; no commit or push.

## Lead acceptance corrections

The lead corrected f32 spacing (4096m -> approximately0.488mm), separated legacy
core bounds from wetland detail, rejected intrinsic-nondeterminism claims about
downsampling, preserved D2 ownership of all core integration tests, and required
persisted-edit invalidation after fine eviction. The resulting memo is decision
input only; no coarse prototype is adopted. Virtual Matter was recorded as a
new reference question, with reuse terms/capability fit explicitly unknown.
