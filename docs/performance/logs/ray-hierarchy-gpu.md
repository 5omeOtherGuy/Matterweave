# Optional packed-occupancy GPU traversal candidate (issue 42)

Worker log for the native GPU half of the bounded M2 traversal experiment. Base
`d37ca73` on `origin/main`; branch `engine/ray-hierarchy-gpu`. Code and test checkpoint
`bf33a96`; the frozen reviewed head is the branch head at handoff and is recorded in the
PR. This log and its two documentation companions are the only changes after that
checkpoint.

**State: optional native GPU candidate GREEN on lavapipe (llvmpipe) with zero
unexplained mismatches. Android execution NOT RUN. No renderer-path selection, no normal
default change, no performance claim and no resolution of the open GPU/Android/cost gates
in issue 42/41.**

## Repair pass (fix58): exact bit-position validation, rebase onto `48c851e`

Independent re-review of PR #58 returned REQUEST CHANGES on one blocking correctness
finding. This section records the repair; it is the durable artifact for that pass. It
changes no renderer default, makes no performance/device/Android claim and does not
touch the frozen branch `engine/ray-hierarchy-gpu` or PR #58.

### Finding: `from_parts` could silently miss solid cells

`HierarchyUpload::from_parts` (`crates/matterweave-render/src/ray_hierarchy_gpu.rs`) took
`shape` as a free `[u32; 3]` and validated only the material/occupancy word counts, the
`u8` material range and the total set-bit count. None of those pin the bit *positions* to
the claimed shape: for an 8x8x8 crop, `[4,4,8]` (blocks `[2,2,1]`) and `[8,4,4]` (blocks
`[1,2,2]`) both give 4 blocks x 4 words = 16 words and identical set-bit totals, so words
packed under one shape passed with the other. The uniform then described a layout the
bits were not packed in, the kernel's bit gate cleared solid cells, and the shader
silently missed voxels -- bounds-safe, no error. That contradicted the module doc's
promise that such a mismatch is "rejected before it can skip a solid cell".

### Dependency-direction constraint that ruled out the suggested fix

The reviewer proposed taking `&OccupancyGrid`/`&HierarchyVolume`, or comparing against
`occupancy.shape()`. None of those compiles here: `OccupancyGrid` and `HierarchyVolume`
live in `matterweave-ray-hierarchy`, and `crates/matterweave-ray-hierarchy/Cargo.toml:10`
declares `matterweave-render = { path = "../matterweave-render" }`. `from_parts` is in
`matterweave-render`, so naming either type there is a circular crate dependency. The fix
had to use only what `matterweave-render` already holds: `materials`, `occupancy` and the
claimed `shape`.

### Fix chosen

Replaced the layout-independent popcount check with the shader's own indexing. For every
crop cell, `from_parts` now computes, under the claimed shape,
`block = cell / shape`, `local = cell % shape`,
`bit = local.x + shape.x*(local.y + shape.y*local.z)`,
`block_index = block.x + block_dims.x*(block.y + block_dims.y*block.z)` and
`word = block_index*words_per_block + bit/32`, and requires that bit to equal
`material != 0`. This is O(cells) -- the same order as the popcount scan it replaces --
allocates nothing and stays inside `matterweave-render`. `HierarchyLayout` keeps
`block_dims = ceil(dimensions/shape)` and `words_per_block = ceil(bits/32)`, so the check
is exactly the packing in `matterweave-ray-hierarchy/src/occupancy.rs` and in the kernel.
The equal-total check is retained as a guard against set bits in padding positions the
kernel never tests (partial blocks, and a block's last word), so no stray bit survives.

### RED-first regression

New test `upload_rejects_equal_wordcount_equal_popcount_shape_mismatch`: an 8x8x8 crop,
one solid cell at `[4,0,0]`, occupancy packed under `[4,4,8]` (word 4, bit 0) and then
claimed as `[8,4,4]` (the same cell's bit under the claimed shape is word 0, bit 4). The
test asserts the two layouts have equal `occupancy_words()`, asserts the packed shape is
accepted, and asserts the claimed shape is rejected.

- Before the fix: `cargo test -p matterweave-render --lib ray_hierarchy_gpu` ->
  `test result: FAILED. 5 passed; 1 failed; 0 ignored` -- the claimed-shape assertion
  failed because `from_parts` returned `Ok`.
- After the fix: the same command -> `6 passed`.

### Rebase onto current main

Branch `engine/ray-hierarchy-gpu-fix58` (confirmed with `git rev-parse --abbrev-ref HEAD`)
was rebased from `d37ca73` onto `origin/main` @ `48c851e`. Three commits replayed; the only
conflict was the expected module-registration collision in
`crates/matterweave-render/src/lib.rs` between PR #57's test-only `moving_lighting_probe`
and this branch's `ray_hierarchy_gpu`. Both registrations were kept in the file's
alphabetical ordering (`moving_lighting_probe` before `ray_hierarchy_gpu`); nothing else
was dropped, reordered or reformatted. After the rebase,
`git diff origin/main HEAD -- crates/matterweave-render/src/lib.rs` is exactly
`+pub mod ray_hierarchy_gpu;` (one insertion), and
`git diff origin/main HEAD -- .../ray_reference.rs .../ray_reference.wgsl` is empty.

The rebase produced new commit SHAs; the original branch `engine/ray-hierarchy-gpu` and
PR #58 are untouched. Repair code checkpoint: `da72d29`.

### Repair-pass verification

Environment: recovery SSD, `RUSTC_WRAPPER=` (no sccache), `CARGO_BUILD_JOBS=1`,
`CARGO_TARGET_DIR=../raygpu-fix58-target`. 13 GB free before and after; `/mnt/bench` was
not used; no native GPU/lavapipe campaign was re-run (no device) and no performance,
thermal, device or Android claim is made.

| Command | Result |
| --- | --- |
| `cargo test -p matterweave-render --lib ray_hierarchy_gpu` (before fix) | 1 failed: `FAILED. 5 passed; 1 failed` |
| `cargo test -p matterweave-render --lib ray_hierarchy_gpu` (after fix) | 6 passed |
| `cargo test -p matterweave-render --lib ray_reference` | 7 passed (retained reference intact) |
| `cargo test -p matterweave-ray-hierarchy` | 32 passed (retained CPU suites intact) |
| `cargo test -p matterweave-ray-hierarchy --example ray_hierarchy_gpu` | 3 passed (classifier controls) |
| `cargo clippy -p matterweave-render -p matterweave-ray-hierarchy --all-targets -- -D warnings` | exit 0, 0 errors (one pre-existing vendored-winit lint) |
| `cargo fmt -p matterweave-render -p matterweave-ray-hierarchy -- --check` | clean |
| `python3 tools/check_docs.py` | PASS: 237 Markdown files, 678 local links, 16 ADRs and 20 requirements |

### Still-open review advisories (NOT fixed this pass)

- **(2) No machine-readable evidence artifact** for the 440-check native GPU run.
- **(3) `../orchestration/deepseek-ray-gpu/result.json` wrongly records all five DoD
  criteria as NOT RUN** while the committed log shows PASS. The log is the truthful
  artifact; the orchestration record is wrong.
- **(4) Vulkan objects leak on mid-`Session::new` failure.**
- **(5) `palette[material]` has no shader-side clamp** and no `robustBufferAccess`.

## Question and answer

Can the CPU `BlockMask` candidate run as a fragment kernel over the same bounded crop and
packed block occupancy, on a real Vulkan device, and return the same hits, materials,
normals and depth as the retained dense reference lineage?

Yes for the tested device and corpora, with the disclosed differences. The kernel walks
the mirrored geometry with the reference's entry/tie/step rules, gates each material read
on the packed bit, and produced 440 checks with 0 failures: 146 exact hits, 0 unexplained
differences, 0 proven tie resolutions, and 288 kernel/CPU agreement misses. The three
documented differences that remain are the CPU `max_distance == 0` query (no fragment
equivalent), crop exclusion (asserted on both sides) and `f32`-vs-`f64` last-bit tie
rounding, for which the classifier is strict and had no cases to classify.

## Actions Taken

1. Read the retained reference (`ray_reference.wgsl`, `ray_reference.rs`), the hybrid
   pipeline users (`indirect.rs`, `reflection.rs`, examples), the merged CPU experiment
   (`matterweave-ray-hierarchy`, its log and repair log), issue 42 and the current
   delivery state before writing code. The comparison chain in this delivery is
   `World::raycast` → CPU `Reference`/`BlockMask` (already proven bit-for-bit on CPU) →
   GPU kernel: the harness asserts `Reference == BlockMask` on every probe's geometry and
   then compares the kernel against that CPU hit.
2. Added `crates/matterweave-render/src/ray_hierarchy_gpu.wgsl`: the reference vertex
   stage and a fragment stage whose geometry, slab entry, plane-tie stepping,
   lowest-axis normals, iteration cap, shading and depth are unchanged, with one block
   cache (`words_per_block <= 16`) refilled once per block entry, no material read inside
   an empty block, and a bit-gated material read otherwise. New set-0 bindings: 3 =
   occupancy words, 4 = 48-byte hierarchy parameters.
3. Added `crates/matterweave-render/src/ray_hierarchy_gpu.rs`: SPIR-V constants, binding
   constants, `HierarchyUniform` (three `vec4<u32>`, no padding), `HierarchyLayout`
   (validated crop/block grid and word counts), `HierarchyUpload` (borrowed, bounded,
   material-range and occupancy-count integrity checks, epoch/revision/seed/origin/
   dimension staleness) and `is_fresh`. Five unit tests cover layout rules (including the
   CPU log's 13x16x20 byte totals), rejection cases, staleness parity and SPIR-V entries.
4. Registered the shader in `matterweave-render/build.rs` and exported the module with one
   line in `matterweave-render/src/lib.rs`. `ray_reference.rs`, `ray_reference.wgsl`,
   `indirect.rs`, `reflection.rs`, `lighting.rs`, the explorer/app and every shared
   authority file are untouched.
5. Added the standalone native example
   `crates/matterweave-ray-hierarchy/examples/ray_hierarchy_gpu.rs` (ash, dev-dependency
   beside `bytemuck`): offscreen 1x1 render per probe, color + depth readback, five
   descriptor bindings, validation-layer compatible. It runs the CPU experiment's targeted
   case classes (tied diagonals on two block shapes, zero direction components,
   inside-solid starts in both projections, thin wall with a one-cell opening on a
   word-boundary shape, negative crop edges, crop exclusion), a 48 fixture x 8 ray seeded
   corpus over all four start classes, and edit/staleness checks with real re-derivation
   and re-upload.
6. Made the existing deterministic fixture generator public
   (`matterweave-ray-hierarchy::fixtures`) so the native gate compares the same corpora
   instead of copying the generator, and recorded the optional example in the crate docs,
   README and `docs/DEVELOPMENT.md`.
7. Implemented the strict classifier: mirrored-`f32` unprojection (including the shader's
   plane stabilization) as the CPU input, a bounded mirrored-vs-analytic ray contract
   check, depth equality through the shared `f32` projection formula, palette-space
   material verification, and a geometric proof for any non-equal hit (candidate cell
   inside the crop with nonzero material, one step per differing axis, the hit point on
   every shared plane, an entry normal naming a face that contains the point). Negative
   controls pin material, face, depth, hit/miss and empty-candidate rejections.
8. Ran the focused checks, the validation-layer run with synchronization validation, and
   the retained `ray_reference_vulkan` example as a regression anchor, then committed the
   code and tests, and wrote this log.
9. Repair pass (fix58): replaced the `from_parts` popcount check with exact per-cell bit
   position validation under the claimed shape (see the repair section above), added the
   RED-first equal-word-count/equal-popcount regression, rebased the branch onto
   `origin/main` @ `48c851e` and resolved the single `lib.rs` module-registration conflict
   by keeping both lines. Re-ran the focused tests, clippy, fmt and `check_docs`; did not
   re-run the native GPU campaign (no device, unaffected by this validation-only change).

## Issues & Friction

- **WGSL operator precedence is not Rust's.** `word & mask != 0u` parses as
  `word & (mask != 0u)` and naga's validator rejected the kernel with an
  `InvalidBinaryOperandTypes` error. Fixed by parenthesizing the bit test; the shader is
  only compiled through naga, so `cargo build` catches this class immediately.
- **The mirrored-ray contract bound was first too tight.** The `f32` inverse projection
  moves the far point by up to ~1.9e-3 world units at `FAR = 40` (`2.5e-4` relative),
  which failed 20 seeded probes against a 1e-3 bound. The bound is now 1e-2 with the
  conditioning stated; the CPU side of the comparison uses the mirrored ray, so this
  bound is a mirroring sanity check, not a hit tolerance.
- **Depth must not be compared as a blanket NDC epsilon.** At the near plane a 1e-3 NDC
  difference is sub-cell, but at the far plane it is several cells. Both sides compute
  `clip.z / clip.w` with the same `f32` formula from the same mirrored ray, so the
  comparison uses 5e-5 NDC. The observed maximum was 7.6e-6; at the corpus's farthest hit
  a one-cell distance error is at least ~2e-4 in depth, so the bound cannot absorb a
  wrong hit. The harness prints both numbers.
- **An 8-bit color tolerance alone cannot prove a material.** The first classifier
  accepted a wrong neighbour material in dim light (ambient-only shading compresses
  palette differences below the color tolerance). The tie/exact material check now
  compares in palette space, dividing out the known light and fog factors, with a
  fail-closed floor when the light scale is too small to verify a material.
- **Two clippy findings and one unsupported literal** were caught by the focused lint:
  `manual_slice_size_calculation`, `neg_cmp_op_on_partial_ord`, and the attempt to mirror
  the shader's `2^-20` with a hex-float literal (Rust has none; `2.0f32.powi(-20)` is
  exact and lint-clean).
- **A dead-code warning** for a test-only helper (`Rendered::background`) appeared in the
  non-test build; it is now `#[cfg(test)]`.
- **No unexpected Vulkan validation or synchronization findings** appeared under
  `VK_LAYER_KHRONOS_validation` with synchronization validation; the harness is a
  single-queue, wait-idle loop by construction, and the run log contains zero
  `Validation Error`/`VUID-` lines.
- **`from_parts` accepted a mismatched `(occupancy, shape)` pair** (review REQUEST
  CHANGES). The word-count and popcount checks were layout-independent, so words packed
  under `[4,4,8]` passed with `[8,4,4]` and the shader silently missed solid cells. Fixed
  with exact per-cell bit positions; see the repair section above.
- **The reviewer's suggested fix does not compile in this crate.** `OccupancyGrid` and
  `HierarchyVolume` live in `matterweave-ray-hierarchy`, which depends on
  `matterweave-render`, so `from_parts` cannot name them; the missing information had to
  be recomputed from the arguments already passed in.
- **Four review advisories remain open and are deliberately not fixed here:** (2) no
  machine-readable evidence artifact for the 440-check native run; (3)
  `../orchestration/deepseek-ray-gpu/result.json` records the five DoD criteria as NOT RUN
  while the log shows PASS (the log is correct); (4) Vulkan objects leak on mid-
  `Session::new` failure; (5) `palette[material]` has no shader-side clamp and no
  `robustBufferAccess`.

## Decisions & Rationale

- **`BlockMask`, not `BlockStep`.** The merged CPU decision adapts the block-mask level;
  the coarse empty-block skip is the more complex candidate and stays out of this slice.
- **The kernel lives in `matterweave-render` with a single module export and no
  `RayUniform` change.** The alternative (a new 240-byte combined uniform) would have
  touched the retained reference's byte layout; a second 48-byte uniform plus a new
  storage binding leaves `ray_reference` untouched.
- **Standalone native example instead of renderer/app integration.** The task's fallback
  applies: small public interfaces are enough to run the comparison, so no production
  architecture, settings, or default path was modified. The example is the only consumer
  of the new module.
- **CPU expectation computed on the shader's own mirrored ray.** Comparing against the
  analytic probe ray would fold camera `f32` error into the hit comparison. Mirroring the
  unprojection in the harness makes every observed difference a traversal-arithmetic
  difference, with a separate bounded check that the mirrored ray still matches the
  analytic probe. This does not independently re-validate the camera contract: that
  contract is the retained reference's, and the mirror is documented as such.
- **Palette-space material verification.** The observed pixel is the palette color times
  one light scalar, blended with fog; inverting those known factors is the strongest
  material check the readback supports, and it fails closed when the light scale cannot
  support a conclusion.
- **Strictness over smoothness.** Non-equal hits are accepted only with the geometric
  tie proof; there is no percentage or count budget for unexplained differences. The
  seeded tie counts are printed; the observed value here is zero, which is itself the
  pinned result.
- **`max_distance == 0` is CPU-only and documented as such.** The fragment walks the
  clipped near/far segment; the reference discards a zero-length segment. The harness
  prints the CPU answer and states that the fragment cannot express the query, instead of
  silently dropping the case.
- **No timing, no DPR, no comparison to the raster path.** The example reports functional
  counts only, and the normal renderer remains the only production path.
- **Exact bit positions, not a new dependency or a shape accessor.** `from_parts` cannot
  take `OccupancyGrid`/`HierarchyVolume` (circular crate dependency) and has no shape but
  its `shape` parameter, so the check recomputes the shader's `(block, bit)` indexing from
  `materials`, `occupancy` and the claimed `shape`. It is O(cells), allocation-free and
  keeps the invariant local to `matterweave-render`.
- **Keep the equal-total check as a padding guard.** Per-cell checks pin every
  crop-visible bit; the retained popcount equality additionally rejects set bits in
  padding positions the kernel never tests, preserving the previous strictness.

## Solutions Applied

- Occupancy word fetch once per block entry with a register-resident cache, matching the
  CPU candidate's access structure; bit test answers the cell, and the material read is
  gated on the same bit.
- Bounded upload: crop rules, block-shape bound, exact material/occupancy word counts,
  `u8` material range, and a solid-cell/set-bit count integrity check, all before any
  Vulkan buffer exists. `HierarchyMemoryStats` bounds the five descriptor payloads; the
  largest corpus fixture uploaded 5264 bytes.
- Staleness: `HierarchyUpload::is_current` plus `is_fresh` mirror the pack's
  epoch/revision/seed/origin/dimension identity; the native edit check clears and replaces
  a cell, asserts the old pack and upload are stale, re-derives and re-uploads, and checks
  that the kernel then misses and later hits material 42.
- Classification: mirrored ray → CPU `Reference == BlockMask` assertion → depth equality →
  palette-space material match → geometric tie proof, with six negative controls.
- Documentation: the shader, module, crate docs, README and DEVELOPMENT entry all state
  the optional status, the binding contract, the bounded buffers, the staleness rule and
  the Android gate.
- Silent-miss closure: `HierarchyUpload::from_parts` walks every crop cell in the shader's
  own indexing and requires the occupancy bit at that cell to match the material, so an
  equal-word-count equal-popcount shape mismatch is rejected; the equal-total check is kept
  as a padding guard.
- Integration: the branch was rebased onto `origin/main` @ `48c851e`; the `lib.rs`
  `moving_lighting_probe`/`ray_hierarchy_gpu` collision was resolved by keeping both
  registrations, leaving the branch's own `lib.rs` delta at one added line.

## Insights

- With the geometry mirrored, lavapipe's `f32` kernel agreed with the `f64` host walk on
  every hit in the corpus (146/146), including tied diagonals, zero direction components,
  inside-solid starts, thin-wall openings and negative crop edges. Max depth delta
  `7.6e-6` NDC; max exact-hit palette-space error `7.5e-3`.
- The seeded corpus never needed a tie classification: the mirrored ray and the dyadic
  fixture inputs keep the recomputed crossings equal in both precisions on this device.
  That is a property of this corpus and device, not a proof for mobile `f32`; the
  classifier and the Android gate remain necessary.
- The comparison's weak link is material discrimination from a shaded pixel, not the
  traversal. Palette-space inversion with a fail-closed floor is a stronger check than the
  retained example's color tolerance, but it is still a model-based check; the exact-hit
  count and the depth bound are what pin cell, distance and material together.
- The harness fails closed on wrong data, not only on wrong geometry: replacing the
  occupancy upload with an all-empty grid produced 146 failures and exit 1, and reverting
  restored 440 checks with 0 failures.
- Counted memory access (CPU experiment) and functional GPU agreement are now separate
  pieces of evidence. Neither says anything about device cost, cache behavior or thermal
  response, and none was measured here.
- Word counts and set-bit totals are not a layout: both are invariant under some shape
  changes, so a check that compares only totals cannot prove the bits the kernel will
  actually test. The exact per-cell check is the cheapest faithful statement of the real
  contract, and it caught a case the totals missed.
- A dependency edge is a correctness constraint on fix design. `matterweave-render` being
  the lower crate meant the unavailable type information had to be recomputed from the
  data already passed in rather than imported.

## Verification performed

Environment: recovery SSD, `RUSTC_WRAPPER=` (no sccache), `CARGO_BUILD_JOBS=1`,
`CARGO_TARGET_DIR=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/ray-gpu-target`.
16 GB free before and after; `/mnt/bench` was not used; no timed benchmark, no release
build and no workspace-wide rebuild.

| Command | Result |
| --- | --- |
| `cargo test -p matterweave-render --lib ray_hierarchy_gpu` | 5 passed |
| `cargo test -p matterweave-ray-hierarchy` | 32 passed (retained CPU suites intact) |
| `cargo test -p matterweave-ray-hierarchy --example ray_hierarchy_gpu` | 3 passed (classifier positive/negative controls, mirrored-ray contract) |
| `cargo test -p matterweave-render --lib ray_reference` | 7 passed (retained reference intact) |
| `cargo clippy -p matterweave-render -p matterweave-ray-hierarchy --all-targets -- -D warnings` | 0 errors (one pre-existing vendored-winit lint) |
| `cargo fmt -p matterweave-render -p matterweave-ray-hierarchy -- --check` | clean |
| `VK_ICD_FILENAMES=.../lvp_icd.json VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation VK_LAYER_ENABLES=VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT cargo run -p matterweave-ray-hierarchy --example ray_hierarchy_gpu` | 440 checks, 0 failures, 0 validation/VUID lines |
| same command without the layers | 440 checks, 0 failures |
| `VK_ICD_FILENAMES=.../lvp_icd.json cargo run -p matterweave-render --example ray_reference_vulkan` | 30 checks, 0 failures (normal reference path unchanged) |
| negative control: the same run with a temporary all-empty occupancy upload, reverted before commit | 146 failures, exit 1 (the harness fails on wrong cases) |
| `python3 tools/check_docs.py` | PASS: 230 Markdown files, 673 local links, 16 ADRs, 20 requirements |

## Native fixture manifest

- Device: `llvmpipe (LLVM 20.1.2, 256 bits)`, Vulkan 1.4, ICD
  `/usr/share/vulkan/icd.d/lvp_icd.json`. Layers: none for the acceptance run;
  `VK_LAYER_KHRONOS_validation` plus
  `VK_LAYER_ENABLES=VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT` for the
  validation run. No physical GPU, no phone.
- Build: host debug profile, workspace `Cargo.lock`, checkpoint `bf33a96`.
- Targeted cases (one render per probe, 1x1 offscreen, both projections where noted):
  `tied-diagonal-corners` (4x4x4 and 4x4x8, 4 rays, ortho+persp), `zero-direction-components`
  (6 rays), `inside-solid-starts` (3 rays, ortho+persp), `thin-wall-opening` (4 rays,
  4x4x8 and 8x8x8), `negative-crop-edges` (5 rays, ortho+persp), `crop-exclusion` (1 ray).
- Seeded corpus: seeds `0..48` of `fixtures::sparse_fixture`, 8 rays per fixture (two each
  of `Center`, `Integer`, `Quarter`, `Outside`), shape 4x4x8, perspective only.
- Observed: 440 checks, 148 hits (146 exact + 2 edit-check hits), 288 misses, 0 proven tie
  resolutions, 0 failures; max depth delta `7.629424e-6`, max exact palette error
  `7.5202957e-3`, max upload 5264 bytes.
- Edit scenario: single cell `[2, 0, 0]`, initial material 41, cleared, then replaced with
  42; the pack and upload report stale after each world edit and the kernel follows the
  re-derived buffers.
- Documented differences printed by the harness: CPU `max_distance == 0` inside-solid
  query (distance 0, zero normal) is not expressible in the fragment; the CPU oracle hits
  the out-of-crop occluder `[-2, 0, 0]` while the crop path and kernel ignore it.

## Delivery

Code and tests checkpoint: `bf33a96`. Repair code checkpoint: `da72d29`. Frozen reviewed
source: the branch head at handoff (recorded in the PR body); the original branch
`engine/ray-hierarchy-gpu` and PR #58 are left exactly as they were. Files:
`matterweave-render` `ray_hierarchy_gpu.{rs,wgsl}` + `build.rs`/`lib.rs` registrations,
`matterweave-ray-hierarchy` example + public `fixtures` + dev-dependencies, `Cargo.lock`,
and the crate README/DEVELOPMENT entries. The repair is additive on the new branch
`engine/ray-hierarchy-gpu-fix58`; the PR is opened for review and is not merged by this
worker; issue 42 stays open.

## Next gate for the Android/device owner

Explicitly NOT RUN here. The device owner holds the phone (OnePlus 13, Adreno 830, Vulkan
1.3.284 per the current STATUS delivery) and the Android instructions are:

1. Port nothing to the app: this candidate is standalone. If the phone functional check
   is wanted, the smallest path is to add a native-only test entry point behind an
   existing debug/diagnostic flag, upload the same two buffers and run the same probe
   corpus, or to trust the host result and go straight to the representative comparison.
2. If it is run: force the mirroring of the camera unprojection on the device too, keep
   the classifier, and expect mobile `f32` tie resolutions that lavapipe did not produce.
   Re-record the classified counts for that device instead of reusing this device's zero.
3. Do not read a performance result from this delivery. The representative-quality
   comparison from issue 42 is the next cost gate and needs its own device, scene, seed,
   build configuration and thermal context.

## Opus review brief

- **Frozen review SHA:** branch head at handoff (code checkpoint `bf33a96`).
- **What to verify:** (1) the kernel is semantically the retained reference plus a
  bit-gated material read — geometry, slab entry, tie stepping, normals, cap, shading and
  depth unchanged; (2) `ray_hierarchy_gpu.rs` is the only new public surface, with the
  single `lib.rs` line and no `ray_reference` change; (3) the upload validation and
  staleness rule really bound the five buffers and reject stale/replacement packs; (4) the
  classifier is strict and its negative controls fail closed, including the palette-space
  floor; (5) the log's counts and tolerances match the code constants and the recorded
  run; (6) no perf/default-path/documentation overclaim, and the Android gate is marked
  NOT RUN.
- **Explicitly out of scope:** device cost, Android execution, DAGs/trees, production
  integration, ADR/STATUS/board edits.

## Limits

- One device (llvmpipe) and one host build; no physical GPU, no phone, no driver matrix.
- The classifier is a model-based check (mirrored projection, golden shading, palette-space
  inversion) with a documented fail-closed floor; it is not a proof over all float inputs
  and it can only verify materials whose light scale supports a conclusion.
- The corpus is deterministic but not exhaustive; the pinned tie count is zero for this
  device and must be re-recorded for another device.
- One block level over a fixed bounded crop; no multi-level hierarchy, compression,
  streaming, moving objects or GPU-side edits.
- No wall-clock, bandwidth, thermal or power measurement was taken.
