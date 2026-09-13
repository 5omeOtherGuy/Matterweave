# D4.3 authoring documentation repair — 2026-09-13

Completion log for the preserved D4.3 documentation slice after the independent
review of its first freeze (`a98553f`). Documentation scope only: this log,
`docs/AUTHORING.md` and `docs/performance/logs/d4-3-authoring.md`. No runtime
code, tests, data or shared coordination files were changed; no phone,
descendants, benchmarks, `/mnt/bench` or external research were used.

- Branch: `engine/shared-services-authoring` (pushed at
  `a98553fcb19f33d62a8fc998982eca7e8bf1eaed`).
- Main at reconciliation: `54ec529e9070efaf7406796dc0ef144a30807b14`
  (PRs #48 detail, #49 ANR, #51 CPU experiment, #52 evidence merged).
- Repair base: merge of `origin/main` into the branch, merge commit
  `be66ff4943fbddff08e3c3b9adc3cdf8b6ed3483`. The merge was clean (no
  conflicts) and no shared history was rewritten; nothing was force-pushed.
- Owned outputs: `docs/AUTHORING.md`, `docs/performance/logs/d4-3-authoring.md`
  and this log. `docs/AUTHORING*` and the existing authoring log are the paths
  introduced by the frozen commit's diff.

## Actions Taken

- Reconciled Git state before writing: verified the checkout was clean, the
  branch was `a98553f` (ahead 1, behind 44 of `origin/main`), fetched and merged
  `origin/main` (`54ec529`) into the branch, and confirmed the merge introduced
  no authoring-file conflicts. No other worktree or branch was touched.
  Ancestry checks against `origin/main` at the time of writing:
  `engine/mesh-lighting-integration` and `phase-a/d3-1-production-lighting`
  are merged; `engine/shared-mobile-settings`, `engine/recovered-dynamic-lighting`,
  `engine/wetland-proxy-lighting` and `engine/shared-audio-integration` are
  unmerged. The guide therefore bounds its shipped-vs-in-flight statement to
  "beyond the surface documented above" instead of claiming all lighting work
  is unmerged.
- Recovered the prior review from
  `../orchestration/authoring-review/result.json` (worker `w_c6f0d2d0`,
  gemini-3.8-flash-high) and preserved its three findings with dispositions in
  the dedicated section below, because that review lives in a private
  orchestration directory outside Git.
- Verified each finding against the merged tree rather than trusting line
  numbers from the review (which was written against `495d9f9`):
  - `crates/matterweave-detail/src/showcase.rs:1923` returns
    `Result<Showcase>` (`crates/matterweave-detail/src/lib.rs:132` defines
    `Result<T>`); `showcase.rs:929-930` is `pub struct Showcase { pub scene:
    DetailScene, ... }`; `crates/matterweave-detail/src/lib.rs:65` exports
    `SHOWCASE_SEED` (`apps/explorer/src/wetland.rs:41` pins its private `SEED`
    to it and reads `built.scene` at `wetland.rs:483`).
  - `apps/explorer/src/lib.rs` `run_desktop` contains no chooser path and no
    `for_chooser`; `VoxelRelayApp::for_chooser` is `voxel_relay.rs:168`;
    switching is `Experience::switch_if_requested` (`experience.rs:164-186`,
    calling `for_chooser` at `:182`); the chooser button in
    `wetland.rs:1024-1035` sets `sandbox_requested` / `voxel_relay_requested` /
    `terrain_lab_requested`. Persistence has `World::save_with_attachment`
    (`persistence.rs:71`), `World::attachment` (`:65`) and `World::load`
    (`:167`) — there is no `load_with_attachment`; `voxel_relay.rs:248-267` is
    the working save/load pattern.
  - `docs/ENGINE_COMPLETION_PLAN.md` states the accepted D4.2 → D4.3 gate as
    "Both samples run without engine forks; functional input, physical
    simultaneous touch, lock/unlock, audible output and persistence observed";
    `docs/ROADMAP.md` M6 asks only for "the minimal animation, audio, UI and
    asset pipeline those samples actually need" and places "a production
    editor" under "Later research and productization".
- Re-verified every other guide reference against merged `main` before editing
  (representative evidence in Validation). No signature in the guide had
  drifted between `495d9f9` and `54ec529`.
- Applied the three corrections: corrected the `build_showcase` sequence in
  `docs/AUTHORING.md`; corrected the sample-registration steps and the
  attachment-load wording; reclassified the authoring log's observed absences
  against the D4.2 gate. Also added the shipped-vs-unmerged and open-acceptance
  boundaries requested for this repair.
- Ran the required checks and one bounded snippet compile; results in
  Validation. `docschecker` (first run) failed only because this log did not
  exist yet; it passes with the log present.

## Prior review findings and disposition

Source: `../orchestration/authoring-review/result.json`, independent read-only
review of `a98553f`; all three findings were verified against the merged tree
and none was dismissed.

1. **`build_showcase` return type and seed constant misrepresented**
   (`docs/AUTHORING.md:67-69` at the freeze). The guide said
   `build_showcase(SEED)` and treated the result as a `DetailScene`, which would
   not compile: the function returns `Result<Showcase>` and `SEED` is a private
   wetland constant, while the exported constant is `SHOWCASE_SEED`.
   **Disposition:** fixed. Step 1 now uses
   `build_showcase(matterweave_detail::SHOWCASE_SEED)?` and `showcase.scene`,
   with the exact two-line snippet; the snippet was compiled against the merged
   crates (Validation).
2. **Sample registration directed chooser wiring to `lib.rs` and misnamed
   attachment loading** (`docs/AUTHORING.md:123, 129-131` at the freeze). The
   guide implied `lib.rs` owns the chooser path and that a `load()` exists via
   `save_with_attachment`. In the source, `lib.rs` only parses the standalone
   CLI flags; the chooser constructor is per-sample (`for_chooser`), invoked by
   `Experience::switch_if_requested`, and triggered from the chooser buttons in
   `wetland.rs`; loading requires `World::load` plus `world.attachment()`.
   **Disposition:** fixed. Step 1 now names `World::save_with_attachment`,
   `World::load` and `world.attachment()`; step 4 separates the `lib.rs`
   standalone flag from the chooser touchpoints in `experience.rs` and
   `wetland.rs`.
3. **Authoring log expanded D4.2 into productization gates**
   (`docs/performance/logs/d4-3-authoring.md:78-113` at the freeze). The log
   labeled a production editor, compressed codecs/asset pipeline, settings UI
   and save-journal migration as "D4.2 gaps" / "remains D4.2 work", which
   misstates accepted scope: those are explicitly deferred productization in
   `docs/ROADMAP.md`, while the D4.2 gate is the two-sample no-fork run with
   functional input, physical simultaneous touch, lock/unlock, audible output
   and persistence observed. **Disposition:** fixed. The section is
   reclassified into: gate evidence already satisfied, gate items still open
   (physical simultaneous touch, lock/unlock), and observed absences that are
   not gates. Nothing was deleted from the historical facts.

## Issues & Friction

- The review's line numbers were written against the old base and had drifted
  by the time `main` advanced 44 commits (for example, `SHOWCASE_SEED` moved
  from `lib.rs:59` to `lib.rs:65`, `showcase.rs` return type to
  `Result<Showcase>` via the crate `Result<T>` alias). Every reference was
  re-checked against merged sources; the findings were still valid, the
  coordinates were not.
- This isolated authoring checkout had no pre-existing Cargo target directory,
  and the other `*-target` directories under the recovery root belong to other
  worktrees. A snippet compile therefore used a newly created, explicitly owned
  target (`authoring-target`), never `/mnt/bench` and never another worker's
  target; `RUSTC_WRAPPER=` disabled the globally configured sccache and
  `CARGO_BUILD_JOBS=1` bounded the build.
- The first `check_docs.py` run failed because the repair log referenced in the
  corrected authoring log did not exist yet. That is the checker working as
  intended; the failure disappeared once this log was written.
- `?` on the guide's two snippets needs one error type that accepts both
  `DetailError` and physics `String` errors, so the throwaway harness defines a
  two-variant error. The documented lines are unchanged; only the surrounding
  harness converts.

## Decisions & Rationale

- Merged `origin/main` instead of rebasing: the branch was already pushed, and
  merging is clean and forward-only. This satisfies "rebase/merge only if clean
  and safe" without rewriting a pushed branch or forcing any push.
- Kept the first-freeze log's failure history instead of deleting it: the
  factual observations (unwired mute/volume, silent standalone runs, no
  animation system, template-based audio, journal abandonment) remain useful
  evidence; only their classification against accepted scope was wrong. The
  repair log is the durable record of the correction.
- Did not name the unmerged settings/lighting branches inside the authoring
  guide. The guide is durable product documentation; naming transient branches
  would rot. It states the boundary (unmerged worker work is not shipped in
  `main`) without branch names. Branch names stay in this dated worker log.
- Scoped the snippet compile to the public authoring surface
  (`matterweave-core`, `matterweave-detail`, `matterweave-physics`) and checked
  the explorer-internal glue (events, `Experience`, chooser, persistence
  call sites) by signature evidence instead. The guide explicitly tells
  authors not to import explorer glue, so compiling it out of tree would not
  validate anything an author can use.
- No STATUS/HANDOFF/board edits; proposed shared-status text is recorded below
  for the lead per this task's ownership rules.

## Solutions Applied

- `docs/AUTHORING.md`:
  - "Composing an instance / scene" step 1 now uses `SHOWCASE_SEED`, explains
    the `Showcase` container, and shows the `showcase.scene` extraction; step 2
    is unchanged and compiles against it.
  - "Registering a new native sample" step 1 uses
    `World::save_with_attachment` for saving and `World::load` +
    `world.attachment()` for loading; step 4 separates the `lib.rs` standalone
    flag from `for_chooser`-in-`experience.rs` with the `wetland.rs` chooser
    trigger; step 5 clarifies that `--slalom` is the flag the new sample adds.
  - "Missing features" is split into real engine absences (animation, importers,
    editor/settings UI, no ABI) and scope/acceptance boundaries (unmerged
    settings/lighting workers; open physical multitouch, lock/unlock and
    audibility; host audio is not device audibility; derived-data limits).
- `docs/performance/logs/d4-3-authoring.md`: repair note pointing at this log;
  the "D4.2 gaps" section reclassified against the accepted exit gate; related
  wording in Decisions/Insights/Validation corrected.
- `docs/performance/logs/d4-3-authoring-repair.md`: this log.
- Delivery: commits on `engine/shared-services-authoring`, pushed, PR opened
  against `main` (see Delivery). No CI wait and no self-merge were performed.

## Validation

- `python3 tools/check_docs.py` → PASS after this log was written:
  `PASS: 228 Markdown files, 671 local links, 16 ADRs and 20 requirements.`
  (first run failed only on the not-yet-written `d4-3-authoring-repair.md`
  link, confirmed before writing).
- `git diff --check` → PASS, no whitespace errors.
- Snippet compile (exact corrected guide snippets, wrapped only in an error
  adapter and a fixture `World` for `Physics`):
  `cd ~/pi/tmp/drafts/authoring-repair-snippets &&
  RUSTC_WRAPPER= CARGO_BUILD_JOBS=1
  CARGO_TARGET_DIR=/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/authoring-target
  cargo check --offline` → `Finished dev profile ... in 1m 16s`, exit 0.
  It type-checks `DetailVolume::new`, `Scale::new`, `set`, `add_prototype`,
  `place`, `Transform::new`/`Yaw::Deg90`,
  `build_showcase(SHOWCASE_SEED)` + `showcase.scene`, and
  `Physics::replace_detail_scene(&scene)`.
- API evidence for the remaining guide claims on merged `main` (targeted
  `grep`/read, no invented API): `DetailScene::{source_version,add_prototype,
  edit_instance,instance_ids,place,prototype_mesh,select_lods,prepare_batches}`
  (`scene.rs:500,504,548,649,657,692,865,954`); `material` constants 10-13,
  20-23, 30-48 and `material_policy` (`detail/src/lib.rs:272-303,341`);
  `World::{revision,get,set,chunk_revision,generate}` (`core/src/lib.rs:80,84,
  91,146,201`), `mesh_chunk` (`mesh.rs:147`), `enable_streaming`/`stream_around`
  (`streaming.rs:23,116`), `raycast` (`ray.rs:20`); `Physics::{new,sync_world,
  teleport,set_flying_eye,step,step_objects,spawn_playground,grab,throw,
  break_body,snapshot,restore}` (`physics/src/lib.rs:174,218,319,351,370,393,
  589,737,778,861,932,955`) and `replace_detail_scene`
  (`detail_collision.rs:444-448`); `Renderer::{upload_chunk,chunk_revision,
  upload_dynamic,replace_static_scene,update_static_instances,render,
  upload_indirect,disable_indirect,upload_reflection,disable_reflection,
  render_with_lighting}` (`render/src/lib.rs:1351-1639`),
  `Sun`/`LightingSettings` (`lighting.rs:6-25`), `Hud` (`hud.rs:15,21,37`),
  `DynamicMeshCache` (`dynamic_cache.rs:22`); `InputService` zone/pointer/
  consume API (`input.rs:91-328`); `GameplayEvent` six variants, `EventQueue`
  capacity 16, `AudioScope` three variants (`audio_service.rs:111-127,185,
  245-256`); `AudioService` methods and limits 32/8/4 MiB/64 at 48 kHz
  (`audio/src/service.rs`, `config.rs:7-26`); `World::save`/`attachment`/
  `save_with_attachment`/`load` (`persistence.rs:61,65,71,167`) and
  `FORMAT_VERSION`/`GENERATOR_VERSION` (`core/src/lib.rs:26,28`);
  `VoxelRelaySave` (`voxel_relay.rs:129-135`), `SavedWetland` with 4096-edit /
  2 MiB bounds (`wetland_state.rs:13,27-35`) and `load_recovering_with`
  (`wetland_state.rs:112`); D4.2 gate and deferred-productization wording
  (`ENGINE_COMPLETION_PLAN.md`, `ROADMAP.md` M6 and "Later research and
  productization"); open multitouch/lock-unlock and audibility claims
  (`STATUS.md`). No runtime code was changed.
- Not run (out of scope, and reported as not run): device/phone acceptance,
  APK builds, benchmarks, workspace test suites, CI. No performance or device
  claim is made anywhere in this repair.

## Delivery

- Merge commit: `be66ff4943fbddff08e3c3b9adc3cdf8b6ed3483`.
- Repair commit (frozen head of the branch after this log): recorded in the
  PR/handoff message as the frozen SHA; the branch was pushed without force.
- PR: opened against `main` from `engine/shared-services-authoring`; review
  belongs to the dispatched independent reviewer. No CI wait, no self-merge.
- Definition of done:

| Criterion | Verification method | Owner | Result |
| --- | --- | --- | --- |
| Review corrections | All 3 verified against real current APIs/log scope | You | DONE — findings 1-3 re-checked and fixed |
| Usable authoring docs | Existing sample extension steps accurate, no invented gates | You | DONE — build_showcase/chooser/attachment steps corrected; D4.2 vs productization separated |
| Checks | Docschecker/diffcheck plus relevant snippet verification | You | DONE — checker PASS, diff check PASS, snippet `cargo check` exit 0 |
| Delivery | Scoped commits pushed, PR opened, frozen review handoff | You | DONE — merge + repair commit pushed, PR opened |
| Independent review | Opus medium separate reviewer before merge | Codex dispatch | PENDING — bounded brief returned |

## Proposed shared-status text (not applied; lead owns STATUS/HANDOFF/board)

Proposed `docs/STATUS.md` addition under the D4.3 documentation work, verbatim
so the lead can apply or adapt it:

> **D4.3 authoring documentation repair (2026-09-13).** Independent review
> (`w_c6f0d2d0`) of the first authoring freeze (`a98553f`) found three
> corrections; all are fixed on `engine/shared-services-authoring` after a clean
> merge of `main` `54ec529`. `docs/AUTHORING.md` now builds the wetland with
> `build_showcase(SHOWCASE_SEED)?` and `showcase.scene`, documents chooser
> registration across `experience.rs`/`wetland.rs`, and documents persistence
> loading as `World::load` + `world.attachment()`. The D4.3 log no longer
> presents a production editor, compressed codecs or save-journal migration as
> D4.2 gates; D4.2's physical simultaneous touch and lock/unlock remain open
> device acceptance. `check_docs.py` and `git diff --check` pass at the frozen
> SHA, and the guide's corrected snippets compile against the merged crates.
> Documentation only: no runtime code changed.
