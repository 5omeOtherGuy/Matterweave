# D4.3 authoring / extension documentation — 2026-09-12

Worker log for primary-plan slice D4.3 (documentation scope only). Base commit
`495d9f97edd3c2e4886b3506de7d69fa60fe6445`, workspace
`/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/authoring`.
Owned paths only: `docs/AUTHORING.md` (new) and this log. No runtime, STATUS,
HANDOFF or board edits. No device, APK, benchmarks, `/mnt/bench`, or external
research. This completes documentation scope only, not D4 functional acceptance
or physical input gates.

> Repair note (2026-09-13): this log's first freeze (`a98553f`) was independently
> reviewed; three corrections were applied and re-verified after merging current
> `main`. Historical findings and dispositions are preserved in
> [d4-3-authoring-repair.md](d4-3-authoring-repair.md).

## Actions Taken

- Read `AGENTS.md`, `docs/REQUIREMENTS.md`, `docs/DEVELOPMENT.md`,
  `docs/ENGINE_COMPLETION_PLAN.md` (D4 Package/M6-A scope) and `docs/STATUS.md`.
- Read actual current interfaces: `crates/matterweave-core/src/{lib,input,
  persistence}.rs`, `crates/matterweave-detail/src/{lib,scene}.rs`,
  `matterweave-physics` / `matterweave-render` / `matterweave-audio` public
  `pub fn/struct/enum/const` surfaces, `apps/explorer/{Cargo.toml,src/
  lib,audio_service,experience,voxel_relay,wetland,wetland_state,main}.rs`.
- Verified every cited signature and constant against source via targeted grep
  (no API invented): `World::{generate,get,set,raycast,save,save_with_attachment,
  load,attachment,chunk_revision,stats}`, `DetailVolume::set`,
  `DetailScene::{add_prototype,place,edit_instance,prototype_mesh,select_lods}`,
  `Transform::new`, `material::*` ids and `material_policy`,
  `Physics::{sync_world,replace_detail_scene,step,grab,throw,break_body,
  snapshot,restore}`, `Renderer::{upload_chunk,chunk_revision,render_with_lighting,
  replace_static_scene,upload_indirect,upload_reflection}`, `Hud::{rect,text}`,
  `AudioService::{register_clip,play,stop_voice,suspend,resume,poll_device}`,
  `ClipSpec::mono`, `PlayOptions::with_gain`, limits 32/8/4 MiB/64,
  `InputService` zone/consume/take API, `GameplayEvent::ALL`,
  `EventQueue::{push,pop,clear}`, `AudioScope::{Wetland,Sandbox,VoxelRelay}`,
  `VoxelRelayApp::{generate_chamber,initial_physics,for_chooser,pop_event,
  clear_events}`, `SavedWetland::{load_recovering_with,save}`,
  `CHAMBER_SEED`/`SHOWCASE_SEED`, `run_desktop` flags.
- Wrote `docs/AUTHORING.md`: prototype/material authoring, authoritative vs
  derived data, scene composition, input/actions/audio wiring, persistence and
  seeded reproducibility, new-sample registration, and missing features. Wetland
  and Relay are both used as source-backed examples of the no-fork rule.
- Ran `python3 tools/check_docs.py` and `git diff --check`; committed the two
  owned files only; no push/PR.

## Issues & Friction

- `audio_service.rs` is ~2000 lines with unit tests interleaved; the module-level
  contract docs (backend selection, silence semantics, unwired mute/volume)
  proved more reliable than sampling test bodies, so examples cite the doc
  contract plus the exact method names.
- `apps/explorer/src/lib.rs` is ~2000 lines; sample registration spans
  `run_desktop` flags, `Experience` switching and per-sample `for_chooser`
  constructors rather than one registry, so the guide lists all three touch
  points instead of pretending a single registration call exists.
- Kept the guide concise by linking [DEVELOPMENT.md](../../DEVELOPMENT.md) for all
  commands rather than duplicating setup/build/gate instructions.

## Decisions & Rationale

- Every code snippet cites its source path and matches current signatures;
  anything beyond that is labeled pseudocode or explicitly marked as a
  hypothetical new-sample sketch. No full asset/editor pipeline, stable ABI, or
  unimplemented animation/service is claimed.
- Reusable crates vs explorer-internal glue are separated in a table up front,
  because the most likely authoring mistake is depending on sample glue
  (`AudioAdapter`, `Experience`, `WetlandApp`) instead of the crate service
  underneath.
- Authoritative/derived, persistence/seed and service-boundary sections carry
  the D4 acceptance-relevant contracts (revision gating, publication deferral,
  atomic saves, generator+seed pinning) with the exact enforcing call sites.
- Observed absences are recorded here as evidence against the D4.2 gate and
  explicitly classified (gate item vs deferred productization) rather than
  repaired: this slice owns no runtime paths and was instructed not to
  implement speculative abstractions.

## Solutions Applied

- `docs/AUTHORING.md` written (new file): seven sections covering the required
  topics, both samples, registration steps for a new native sample, and a
  missing-features list.
- This log written with the observed-absence classification list below and
  validation results.

## Observed absences vs D4.2 scope (corrected after review — 2026-09-13)

Correction note: the first version of this section labeled deferred
productization (editor, codec/asset pipeline, settings UI, journal migration)
as D4.2 gaps. That was wrong. The accepted D4.2 exit gate is
`ENGINE_COMPLETION_PLAN.md`: "Both samples run without engine forks; functional
input, physical simultaneous touch, lock/unlock, audible output and persistence
observed." The items below are classified against that gate, not against
commercial engine feature parity.

D4.2 gate evidence already satisfied by merged `main`:

- Both samples run without engine forks on the shared crates (wetland: detail
  scene plus edit journal; Relay: core `World` chamber plus attachment save).
- Functional input and persistence are demonstrated by `matterweave-explorer`
  tests and by the Relay physical device gate recorded in STATUS.
- Audible output is accepted on the owner's device confirmation (STATUS.md);
  host counters remain non-evidence.

D4.2 gate items still open (device acceptance, not engine API gaps):

- Physical simultaneous multi-finger use and lock/unlock are untested (STATUS:
  ADB gestures here are sequential; unit tests cover concurrent touch roles,
  which is not physical multi-finger validation).
Observed absences that are not D4.2 gates:

1. **No animation system.** Both samples are fully functional without one; the
   ROADMAP M6 phrase "the minimal animation … those samples actually need"
   resolves to zero for these two. A future animated sample adds new engine
   scope rather than closing a D4.2 defect.
2. **Mute/volume are implemented but unwired.** `AudioAdapter::set_muted` /
   `set_volume` exist (`apps/explorer/src/audio_service.rs`), but no caller
   outside that module's tests references them, and the module docs state no
   sample UI exposes them. Authoring a settings menu is deferred
   productization, not a D4.2 exit criterion.
3. **TerrainLab has no audio participation.** `terrain_lab.rs` contains zero
   references to `EventQueue`, `GameplayEvent` or `AudioScope`, and
   `experience.rs` maps the lab to the event-free `AudioScope::Sandbox`. The
   lab is an internal developer harness, not one of the two required samples.
4. **Standalone sample runs are silent by construction.** Design fact, not a
   defect: only the chooser/`Experience` path owns and drains an audio queue.
5. **No external asset pipeline and no compressed audio.** All audio clips are
   code-synthesized (`synth_clip`); only 48 kHz f32 mono/stereo is accepted,
   with no resampling and no compressed formats (`crates/matterweave-audio`).
   Detail geometry is authored cell-by-cell in code (`showcase.rs`, `flora.rs`,
   `fixtures.rs`). This is the accepted authoring path for both samples; an
   importer/codec pipeline is deferred productization (ROADMAP "Later research
   and productization").
6. **No level/material editor or settings UI.** Follows from 2 and 5; deferred
   productization, not a D4.2 gate.
7. **Changed generator layouts abandon old journals instead of migrating.**
   `old_showcase_sessions_are_not_replayed_on_changed_layout`
   (`wetland_state.rs` tests) pins the behavior: old instance edits get a fresh
   recovery slot, never a migration. Correct conservatism; journal migration
   tooling is deferred productization, not a D4.2 gate.

## Insights

- The engine's hardest authoring rule is already enforced by types and tests
  (revision-gated uploads, publication deferral, atomic saves with caps,
  generator+seed rejection); the guide's main job was pointing authors at the
  existing enforcement sites rather than restating rules.
- The audio seam is the cleanest extension example in the tree (plain-data
  vocabulary, one owner, bounded queue, exhaustive-match compiler guidance for
  new variants) and the natural template for any future animation-event or
  settings plumbing — but that plumbing itself remains deferred productization,
  not this slice.

## Validation

- `python3 tools/check_docs.py`: PASS at first freeze — `213 Markdown files,
  645 local links, 16 ADRs and 20 requirements.` (re-validated after the
  correction in [d4-3-authoring-repair.md](d4-3-authoring-repair.md).)
- `git diff --check`: PASS (no whitespace errors).
- Referenced identifiers/paths: validated by targeted `grep`/`sed` reads
  listed under Actions Taken before writing; no invented API.
- `git status` before commit showed only the two owned files modified;
  commit SHA recorded below. No push or PR per instructions.
