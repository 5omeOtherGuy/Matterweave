# Wetland proxy lighting corrective review repair — 2026-09-12

Repair of the supported findings in the independent Opus review of PR54
(`../orchestration/opus-lighting54-review/result.json`), frozen source
`76043d394f106d550ebb43588db3a5c1310c9229` plus docs
`beda5ad258de843ff00955e5a2b1449df0099166`. Scope: findings 1 and 2 (code) and
findings 3 and 4 (docs), with finding 5 recorded as an Android-gate check.
Owned writes: `apps/explorer/src/wetland_lighting.rs` and this log only;
`crates/matterweave-render/**`, `wetland.rs`, `lib.rs`, the shaders, every other
app module and `docs/STATUS.md` are untouched. No descendants, phone or
`/mnt/bench` access. No long wetland replay was re-run: the corrections are
error/state-path only, and the frozen host smoke and replay evidence stay
attributed to the old source as recorded below.

## Actions Taken

- Read the frozen `wetland_lighting.rs`, the Opus review, and the renderer
  contracts it cites (`indirect.rs` `MeshProxy::build` / `IndirectVolume::new` /
  `set_mesh_proxy` / `valid_for`, `reflection.rs`
  `ReflectionVolume::pack_with_mesh` / `valid_for_scene`) before changing code.
  Verified each finding against the frozen source rather than the review prose.
- Confirmed finding 1 statically and then with an experiment. Frozen `step`
  (`:358-376`) updated `build_failed`, `built_dynamic` and `attached_*` before
  calling `attach`; `attach` exits through `?` at `IndirectVolume::new`
  (`:505-513`) or `pack_with_mesh` (`:529-538`), before `set_mesh_proxy`
  (`:541-543`). A failed attach therefore left the superseded volume attached
  with `build_failed == false`, and the next no-rebuild frame republished it.
- Repaired `step`: an attach error now drops the volume, palette, reflection
  pack, staleness flag and attached-counters, latches `build_failed`, and returns
  the error. `built_dynamic`, `proxy_rebuilds`, `attached_cells`,
  `attached_pool_meshes`, `proxy_rebuilt` and `rebuild_ms` are recorded only
  after a successful attach. The successful transaction path, the publication
  ordering after `upload_dynamic`, and the no-rebuild republish-from-cache path
  are unchanged.
- Repaired `attach`: the reflection pack is rebuilt when
  `reflection.is_none() || reflection_stale`, and a successful repack clears the
  flag. A stale pack whose digest is unchanged therefore clears with exactly one
  proxy rebuild instead of forcing a rebuild and withholding reflection forever.
- Added three host regressions plus a test double that refuses to let a retired
  representation pass unnoticed: `FakeSink` now records a violation when either
  publication carries a digest the test marked retired, and a test-only
  `AttachFault` injects the volume-allocation or reflection-pack failure at the
  two real `attach` call sites. `AttachFault`, the field and both checks are
  `#[cfg(test)]`, so production builds contain no fault hook.
- Corrected the module's "largest axis-aligned box" claim (finding 4) and added
  the motion-starvation disclosure (finding 3) to the module header, including
  the open moving-body lighting requirement.
- Ran the red/green checkpoints, the focused suite, the full explorer lib suite,
  scoped clippy, `rustfmt` and `python3 tools/check_docs.py`. Commands and
  results are below; no device, timing or thermal claim is made.

## Issues & Friction

- **Finding 1 reproduced exactly as reviewed.** With only the production repairs
  reverted (new tests and the fault hook kept), the allocation-failure regression
  panicked with `["republished a retired representation", "republished a retired
  representation"]` — both the superseded indirect volume and its stale mirror
  pack reached the sink after the failed attach. The pack-failure regression
  panicked with `["republished a retired representation"]`. The old code's
  one-frame `reflection_stale` self-heal does not prevent that first publication.
- **Finding 2 reproduced as reviewed.** In the same reverted run the
  unchanged-digest stale-pack regression panicked at "the repacked bake must be
  valid for the current source": the stale flag was never cleared, reflection
  stayed withdrawn, and the proxy was rebuilt on every frame.
- **The frozen `build_failed` comment did not match the gate.** It promised a
  retry on "the next accepted install or dynamic change"; the rebuild gate
  (`!self.build_failed && ...`) only retries on the next accepted install. This
  repair latches attach failures the same way and the field comment now states
  the actual contract. Adding a dynamic-change retry is a behavior change the
  review did not request and was left out; the trade-off is recorded under
  Decisions.
- **Finding 3's cost is real and larger than the log's framing.** With
  `UPDATE_BUDGET { rays: 1024 }` and two rays per sample over 16 samples per
  face, a frame advances at most 512 face samples; the module's own tests budget
  `CONVERGE_FRAMES = 400`. Every frame in which a body crosses a cell boundary
  withdraws the publication, so GI is off for the whole motion and for the
  reconvergence afterwards, not just for one frame.
- **Finding 4 was a real false claim.** The frozen comment and the original log
  called 4000 cells the residency cap. `MAX_FACE_SLOTS = 24_576`
  (`indirect.rs:26`) allows 4096 cells and `MAX_REFLECTION_AXIS = 64`
  (`reflection.rs:64`) allows 16x16x16 = 4096, so the box is an authored choice,
  not a maximum.
- **Finding 5 is unexercised, not disproven.** `MAX_MESH_PROXY_TESTS = 4 Mi`
  (`indirect.rs:35`, enforced at `:314`) is far above the 39-mesh/1793-cell
  clearing evidence, and no test or host run covers a dense Source-LOD box. It
  stays an Android-gate item; raising the constant is not part of this slice.
- The fault hook is test-only state, not a production seam: the two failures it
  injects are allocation/pack errors that cannot be triggered deterministically
  on the host, and the review itself called them plausible only under device
  memory pressure.

## Decisions & Rationale

- **Attach failure is a build failure plus a dropped representation.** The
  review offered "set `build_failed`, or `self.volume = None`"; both are applied.
  The latch bounds retries, and dropping the volume/reflection makes the
  invariant structural: with no attached representation there is no state the
  no-rebuild path could publish. `attached_cells`/`attached_pool_meshes` are
  zeroed so logging cannot claim a representation that no longer exists.
- **Recovery waits for the next accepted install, matching failed builds.**
  Rendering the retired proxy is the bug; rebuilding it again immediately is an
  allocation storm under exactly the memory pressure that caused the failure.
  A detail install (camera/LOD/edit) retries the build; a body-only move does
  not, the same suppression the frozen code applied to failed builds. The
  motion-withdrawal disclosure covers the user-visible consequence until the
  Phase B moving-body requirement lands.
- **A stale pack with an unchanged digest is repacked from the rebuilt proxy.**
  `IndirectVolume` keeps its `MeshProxy` private, so a correct repack needs the
  freshly built proxy; that is one proxy rebuild that clears the flag, instead of
  a rebuild every frame with reflection permanently withdrawn. The pack bake is
  what actually depends on the world revision/seed; the proxy digest does not.
- **The test double, not the scheduler, owns "retired".** `FakeSink` records the
  digest the test declared retired and flags either publication carrying it,
  which turns the invariant into an observable contract. The test-only
  `AttachFault` injects at the real call sites in `attach`, so the state machine
  is exercised through the same `Result` path production uses.
- **Docs corrections stay in the module header and this log.** Finding 3 is a
  disclosure, not a redesign; finding 4 is a corrected superlative. No renderer
  API, shader, app module or shared status document was touched, and no ADR is
  proposed: this is consumption of accepted engine APIs (ADR-0014/ADR-0015).
- **No full M4 closure is claimed.** Continuous moving-body lighting remains an
  open functional requirement; this slice only makes the failure path safe and
  states the starvation honestly.

## Solutions Applied

Changed files: `apps/explorer/src/wetland_lighting.rs` (production:
`step`/`attach`/field docs; tests: `AttachFault`, `FakeSink::retired_digest`,
`body_at`, three regressions) and this log.

Every Cargo command used
`RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=../wetland-lighting-target`.

### Verification (this repaired source)

- RED checkpoint (production repairs temporarily reverted, tests and fault hook
  kept):
  `cargo test --locked -p matterweave-explorer --lib wetland_lighting`
  -> `13 passed; 3 failed`, with the panics quoted under Issues & Friction.
- GREEN focused suite: same command on the repaired source
  -> `16 passed; 0 failed; 183 filtered out` (13 pre-existing + 3 new).
- Full explorer lib suite: `cargo test --locked -p matterweave-explorer --lib`
  -> `198 passed; 0 failed; 1 ignored`.
- Scoped clippy:
  `cargo clippy --locked -p matterweave-explorer --all-targets -- -D warnings`
  -> exit `0` (only the pre-existing `vendor/winit` notice).
- `rustfmt --edition 2021 --check apps/explorer/src/wetland_lighting.rs`
  -> exit `0`.
- `python3 tools/check_docs.py` ->
  `PASS: 221 Markdown files, 655 local links, 16 ADRs and 20 requirements`,
  exit `0` (includes this log).

| Criterion | Verification method | Owner | Result |
| --- | --- | --- | --- |
| Error contracts repaired | Failed attach cannot publish retired proxy; same-digest stale reflection repacked | worker | **PASS (host)** — dropped representation + latched failure; repack clears `reflection_stale` |
| Meaningful regressions | Red/green error/state tests, scoped checks | worker | **PASS (host)** — 3 new regressions red on reverted production repairs, green on the repair; explorer lib 198 passed / 0 failed / 1 ignored |
| Honest limits | Motion starvation / box capacity / dense gate corrected docs | worker | **PASS** — module header corrected and disclosed; dense box check handed to Android |
| Delivery | Commit push 54, frozen review SHA, this log | worker | **PASS** — see Delivery snapshot |
| Independent review / Android | Separate Opus then DeepSeek device worker | Codex dispatch | **NOT RUN** — outside this writer's scope |

### Finding disposition (for the Opus corrective review)

| Finding | Disposition | Evidence |
| --- | --- | --- |
| 1. `attach()` failure republishes the superseded proxy | **Repaired (code).** Attach error drops volume/reflection/palette/counters, latches `build_failed`, and the frame path withdraws; counters and fingerprint are recorded only on success. | Frozen `wetland_lighting.rs:358-376,495-546`; RED `["republished a retired representation", ...]`; two new tests. |
| 2. `reflection_stale` never clears for an unchanged digest | **Repaired (code).** Repack condition is now `is_none() \|\| reflection_stale` and a successful repack clears the flag. | Frozen `:528-539`; RED "the repacked bake must be valid"; `a_stale_pack_with_an_unchanged_digest_repacks_without_perpetual_rebuilds`. |
| 3. Cell-crossing body motion starves GI | **Disclosed (docs).** New "Scope and limitations" bullet states per-frame withdrawal, `UPDATE_BUDGET` reconvergence and the open moving-body requirement; the digest-gate invariant now says it absorbs camera/LOD churn only. No dynamic-lighting redesign, per scope. | Module header; budget arithmetic in Issues & Friction. |
| 4. "Largest box that fits" is false | **Corrected (docs).** 4000 cells is an authored aspect ratio below both caps (4096 cells each); the module comment and this log say so. The frozen log text is preserved as historical evidence, with the correction here. | `indirect.rs:26`, `reflection.rs:64`; module header. |
| 5. Dense-box `MAX_MESH_PROXY_TESTS` unexercised | **Accepted (Android handoff).** Explicit dense Source-LOD box check added below; not run here, and the constant is not changed. | `indirect.rs:35,314`. |

### Preserved original evidence (frozen `76043d3`, not re-attributed)

The original log `docs/performance/logs/wetland-proxy-lighting.md` is unchanged
and remains the evidence of the frozen source. Its claims stay attached to
`76043d3` and are neither re-run nor re-attributed to this repair:

- 13 `wetland_lighting` tests, 195 explorer lib tests, 107 render tests; host
  llvmpipe smoke converging to `cells=1793 pool_meshes=39
  digest=Some(17984592732044421731)`, `gi=false -> gi=true`; the 600-frame
  camera/LOD replay (`cells 1793 -> 1801 -> 1805`, `rebuild_ms` 48-54 ms debug,
  its own settle-gate `FAIL`); all host-only, no device or timing claim.
- The original log's line "4000 cells = the indirect face-residency cap" is
  wrong and superseded by finding 4 above; it is left in place because it is
  part of the frozen delivery record.

### Android functional handoff

For the DeepSeek device worker on the frozen repair SHA; nothing below was run
here. Standard pass first: stationary converge, walk/LOD converge, one
break/place republish, one body-move republish, lifecycle resume.

- **Dense Source box `MAX_MESH_PROXY_TESTS` check (finding 5).** Stand at the
  destruction clearing and walk the detail selection to its densest in-box state
  (Source LOD). Watch logcat for the frame path's
  `Wetland proxy lighting: Mesh proxy exceeds the triangle/cell test budget`
  line. If it appears, GI and reflection are withheld until the selection moves;
  record the selection, `cells`, `pool_meshes`, digest and the log line, and file
  it as a dense-box finding. Do not raise `MAX_MESH_PROXY_TESTS` in this slice.
- **Motion-withdrawal observation (finding 3).** Move a body across cell
  boundaries continuously and record `gi=false`/reconvergence transitions and the
  per-rebuild `rebuild_ms` at density. The withdrawal is disclosed behavior, not
  a regression to fix here.
- **Attach-failure path (finding 1).** Not device-forcible; the two host
  regressions cover it. If the allocation/pack error ever appears on device,
  capture the `Wetland proxy lighting:` line and the surrounding memory state.
- **Open follow-up functional requirement.** Continuous moving-body lighting
  (publication that survives per-frame footprint churn, or a smaller
  republish interval) remains open Phase B work. This repair does not close M4.

### Delivery snapshot

- Repair source commit, frozen for corrective review:
  `28d30553606f01bdd5d38e6240782d6da552beae` (code, tests and this log in one
  commit), pushed to `engine/wetland-proxy-lighting`, PR54. This docs-freeze
  commit is the only commit after it.
- Frozen review baseline: `76043d394f106d550ebb43588db3a5c1310c9229` + docs
  `beda5ad258de843ff00955e5a2b1449df0099166`.
- The repair source stays frozen after this handoff; no device, timing or
  thermal claim is made, and no CI result is awaited or claimed here.

## Insights

- A validity flag is not a coverage proof: `upload_indirect` compares the
  caller's digest to the volume's, so a stale volume with a stale digest passes
  every renderer check. Withdrawal must happen where the source identity moves,
  not at the upload boundary.
- "Self-heals next frame" still means one frame of retired geometry is drawn.
  The failure path has to make the retired representation unpublishable, not just
  short-lived.
- Latching a failure without dropping the stale object is only half a fix; the
  explicit drop is what makes the no-rebuild path unable to publish retired
  geometry, and it also gives the memory back under the pressure that caused the
  failure.
- A stale bake and a stale proxy are different things. The reflection pack keys
  on the world revision and seed; the indirect digest keys on occupied cells.
  Repacking from the rebuilt proxy lets an unchanged footprint clear its stale
  bake with one build instead of a rebuild loop.
- The disclosed motion cost is the same budget that makes the feature cheap:
  bounding per-frame work is what turns a body move into many frames of
  withdrawal. This is a product trade-off for Phase B, not a tuning accident.
