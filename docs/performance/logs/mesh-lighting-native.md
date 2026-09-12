# Engineering log mesh-lighting-native

Worker slice: native positive MeshProxy lighting publication diagnostic (D3.1/D3.2).
Base `80a89c5` (lead mesh upload API plus independently reviewed D3.2 completion
gate). No push, no PR: integration, independent review and Android are the lead's
row and are not claimed here.

Owned writes: `apps/explorer/src/mesh_lighting_check.rs` (new),
`apps/explorer/src/lib.rs` (module/CLI/Android-registration hunks only),
this log. Every other path is read-only from this slice; the renderer needed no
correction, so there is no correction proposal. No overlap with the concurrent
streaming worker's `lib.rs` tick block (untouched; verified in the diff).

## Actions Taken

- Read `AGENTS.md`, `engine_check.rs`, `reflection_check.rs`,
  `destruction_check.rs`, `detail_check.rs`, `pacing_check.rs`, the
  `MeshProxy`/`IndirectVolume` contract in `crates/matterweave-render/src/indirect.rs`,
  the `ReflectionVolume`/oracle contract in `reflection.rs`, and the production
  publication entries `upload_indirect`/`upload_reflection` in
  `crates/matterweave-render/src/lib.rs` before writing code.
- Implemented `apps/explorer/src/mesh_lighting_check.rs`: a bounded 5-phase
  one-shot diagnostic. The fixture (sunlit floor, mirror receiver column, one
  unit-cube prototype from the engine's own voxel mesher, two `StaticInstance`
  placements plus a static control) lives in memory; the proxy is built from the
  same prototype/instance slices uploaded via `replace_static_scene`.
- Phase plan, all demonstrated with real presentation and CPU-oracle response:
  0 proxy-less world-only lighting rejected while mesh geometry is resident;
  1 matching-proxy GI/reflection publication with red-dominated indirect faces,
  mirror-ray object hit plus green shaded response, and control holds;
  2 real instance move (cell `[0,0,0]` to `[0,2,0]`, new digest) invalidates
  lighting and stale packs are rejected under the new digest;
  3 recompute then republish under the move identity;
  4 geometry removed, world-only `None` publication succeeds.
- Registered the gate minimally in `lib.rs`: `mod mesh_lighting_check`,
  desktop `--mesh-lighting-check` (report beside the save path, nonzero exit on
  failure), Android `engine-check.txt` marker `mesh-lighting`, help text.
- Lifecycle handling reuses the `destruction_check` pattern: phase mutations run
  once per phase (`prepared` guard, idempotent full-scene replace so resume is
  safe), `Retry`/zero-size draws present nothing and are never counted,
  suspend releases GPU state and preserves phase counts, resume re-applies the
  current phase without duplicating report lines (evidence is consumed once at
  phase completion).

## Issues & Friction

- First `cargo test` failed on `Ok(())` against `upload_reflection`, which
  returns `ReflectionUploadStats` (two sites). Fixed with `Ok(_)`.
- Scoped strict Clippy failed on three `!(float_cmp)` assertions
  (`neg_cmp_op_on_partial_ord`) and one `#[cfg(test)]` helper impl placed after
  the test module (`items_after_test_module`). Rewrote the comparisons and moved
  the helper into the main impl block.
- A clipped `tail | echo $?` pipeline masked the real Clippy exit code on two
  runs (reported `0` while errors existed). Per coordination, all reruns use
  `set -o pipefail` with `${PIPESTATUS[0]}`; the masked runs were repaired by
  rerunning Clippy to a genuine exit `0`, not by trusting the masked status.
- `rustfmt --check` on owned files needed one pass (import collapse, small
  function shapes); one `lib.rs` module-ordering nit fixed. Repo-wide
  `cargo fmt --check` still reports pre-existing drift in `engine_check.rs`,
  which is outside this slice and was left untouched.
- Host run printed `vulkan: No DRI3 support detected` under Xvfb but presented
  all 30 frames on llvmpipe regardless; treated as environment notice, not a
  failure.

## Decisions & Rationale

- Reuse, don't build: phase accounting and HUD overlay follow
  `destruction_check`; probe geometry and oracle thresholds follow the
  `indirect_tests` D3.1 probe; publication goes through the unmodified
  production `upload_indirect`/`upload_reflection` entries so rejection strings
  and disable-on-reject behavior are the real ones.
- Correctness evidence is the CPU oracle (indirect samples, reflection
  hit/material/shade, bit-identical controls), never the `enabled` flags alone;
  flags are recorded per phase but asserted only alongside oracle values.
- Always `replace_static_scene` (never `update_static_instances`) so every
  phase application is idempotent and resume-safe without branching.
- No performance measurement of any kind: fixed 6/60 host/Android frame quotas
  exist only to present each phase for lead capture, not as timing evidence.
- Sub-cell digest identity and in-cell self-hit oracle miss stay as documented,
  unit-pinned limits; no raster workaround was added.

## Solutions Applied

- `MeshLightingCheck` with injectable `advance(prepare, draw)` callbacks, so
  Retry/resume accounting is covered by deterministic driverless unit tests.
- Nine focused CPU tests: world-only darkness vs red-dominated proxy response
  with digest reproducibility; move-lowers/control-holds; reflection
  rest-hit/lifted-miss/identical-control plus stale-digest refusal;
  sub-cell-identity pin; self-hit-miss pin; overlay labels; Retry accounting;
  suspend/resume accounting; save isolation.
- Full package suite, scoped strict Clippy, owned-file fmt, docs checker, and
  one host software-Vulkan functional run all pass with recorded real exit
  codes (below).

## Insights

- A masked pipeline exit (`tail` swallowing cargo's status) is a process bug
  with the same shape as the stale-publication bug this diagnostic gates:
  a downstream consumer accepting an identity (exit `0`) it never verified.
  Capture the producer's status explicitly.
- The D3.2 completion gate plus the lead's digest-checked publication entries
  made this slice small: rejection, invalidation and supersession refusal all
  come from production code paths, so the diagnostic is mostly fixture plus
  oracle assertions.

## Verification (exact commands, real exit codes)

All with `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1
CARGO_TARGET_DIR=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/mesh-native-target`:

- `cargo test -p matterweave-explorer --lib --locked` → 166 passed,
  1 ignored, exit `0` (includes the 9 new `mesh_lighting_check` tests).
- `cargo clippy -p matterweave-explorer --all-targets --locked -- -D warnings`
  → 0 errors, exit `0`; the single remaining warning is the pre-existing
  vendor/winit `function_casts_as_integer` notice, untouched.
- `rustfmt --edition 2021 --check` on both owned files → clean, exit `0`.
  Repo-wide `cargo fmt --check` still flags pre-existing `engine_check.rs`
  drift (not owned, not touched).
- `python3 tools/check_docs.py` → PASS: 212 Markdown files, 639 local links,
  16 ADRs and 20 requirements, exit `0`.
- Host functional run (the one validation):
  `timeout 300s xvfb-run -a cargo run --locked -p matterweave-explorer -- --mesh-lighting-check --save "$smoke_dir/world.json"`
  → exit `0`; software Vulkan `llvmpipe (LLVM 20.1.2)`; all 5 phases ×
  6 presented frames; report `PASS mesh-lighting` with rest digest
  `5068635860049478035`, lifted digest `672966773062200507`,
  `total_presented=30 suspends=0 resumes=0`.

## Review correction (independent review, lead-verified findings)

Two real scoped issues, both fixed in `mesh_lighting_check.rs` only
(renderer and `lib.rs` registration untouched):

1. **Missing world raster.** `create_renderer` never uploaded
   `self.world.mesh()`: the CPU floor/receiver existed in the lighting
   volumes but was absent from raster geometry. Every renderer creation now
   uploads the authoritative fixture world (including resume) with
   fixture-palette colours, before any phase lighting publication. `World::mesh`
   paints engine-default colours, so each vertex is recolored through its
   source cell. The naive `floor(p - n * eps)` lookup mis-resolves
   voxel-boundary edge vertices into air (caught by the new test on cell
   `[-5, 0, -4]`); the shipped `source_cell` helper instead tries both
   adjacent cells on boundary axes and keeps a solid cell whose face along the
   vertex normal is actually exposed, naive cell first, deterministic. Exact
   for this fixture, where ambiguous edges are same-material.
2. **Mislabeled evidence.** Phase 1 `cells={}` formatted
   `pack.material_at(OBJECT_CELL)` (a material id) under a count label. Now
   `describe_proxy` reports the digest plus the actual `occupied_cells`
   (2 for the two unit-cube instances), pinned by a unit test.

Host evidence now proves both geometries on GPU: `world geometry uploaded:
1008 vertices 1512 indices (fixture palette)` at creation, and every phase
summary carries `gpu mesh_bytes=43376 static prototypes=1 instances=2
vertices=24 indices=36`; phase 4 after the instance clear reads
`mesh_bytes=42336 static scene cleared`, i.e. the world raster is retained
while only instances were removed.

Verification (same env, original `mesh-native-target`, lead-confirmed free):

- `cargo test -p matterweave-explorer --lib mesh_lighting_check --locked` →
  11 passed, exit `0` (9 prior + recolor palette + `describe_proxy` label).
- `cargo clippy -p matterweave-explorer --all-targets --locked -- -D warnings`
  → 0 errors, exit `0` (only the pre-existing vendor/winit notice).
- `rustfmt --edition 2021 --check` on the owned file → clean, exit `0`.
- `python3 tools/check_docs.py` → PASS (212 files, 639 links), exit `0`.
- Host functional run `timeout 300s xvfb-run -a cargo run --locked
  -p matterweave-explorer -- --mesh-lighting-check --save "$smoke_dir/world.json"`
  → exit `0`, llvmpipe, all 5 phases x 6 presented frames, report PASS with
  rest digest `5068635860049478035`, lifted `672966773062200507`,
  `total_presented=30`. Full app suite not repeated: only the owned module
  changed since `c043ed5`, whose full-suite pass stands.

## Open gates (not run, lead-owned)

Independent review, Android run, and merged delivery. No completion claim is
made here.
