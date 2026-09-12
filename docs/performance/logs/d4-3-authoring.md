# D4.3 authoring / extension documentation — 2026-09-12

Worker log for primary-plan slice D4.3 (documentation scope only). Base commit
`495d9f97edd3c2e4886b3506de7d69fa60fe6445`, workspace
`/home/someotherguy/Documents/ChatGPT/Matterweave-recovery-20260912/authoring`.
Owned paths only: `docs/AUTHORING.md` (new) and this log. No runtime, STATUS,
HANDOFF or board edits. No device, APK, benchmarks, `/mnt/bench`, or external
research. This completes documentation scope only, not D4 functional acceptance
or physical input gates.

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
- D4.2 gaps are recorded here as evidence, not repaired: this slice owns no
  runtime paths and was instructed not to implement speculative abstractions.

## Solutions Applied

- `docs/AUTHORING.md` written (new file): seven sections covering the required
  topics, both samples, registration steps for a new native sample, and a
  missing-features list.
- This log written with the D4.2 gap evidence list below and validation results.

## D4.2 actual gaps discovered (evidence, not implemented)

Scope reference: D4.2 = "only missing minimal animation/UI/assets and shared
persistence/seeded behavior" (`ENGINE_COMPLETION_PLAN.md`, remaining slices).

1. **No animation system exists.** `docs/ROADMAP.md` still lists "the minimal
   animation … those samples actually need" as to-add; `grep -rni animat
   crates apps` returns no engine or sample animation module. Motion today is
   physics bodies and placement transforms only.
2. **Mute/volume are implemented but unwired.** `AudioAdapter::set_muted` /
   `set_volume` exist (`apps/explorer/src/audio_service.rs`), but no caller
   outside that module references them (`grep set_muted|set_volume
   apps/explorer/src` excluding `audio_service.rs` is empty), and the module
   docs state "no sample UI exposes them yet … nothing drives these methods in
   production."
3. **TerrainLab has no audio participation.** `terrain_lab.rs` contains zero
   references to `EventQueue`, `GameplayEvent` or `AudioScope`, and
   `experience.rs` retires the lab scope to `AudioScope::Sandbox` (which
   "triggers no events today"). A lab action is therefore silent even when the
   shared adapter is running.
4. **Standalone sample runs are silent by construction.** `voxel_relay.rs` and
   `experience.rs` module docs both state a sample launched directly
   (`--voxel-relay`) has no audio owner and its bounded queue is never drained.
   Only the chooser/`Experience` path produces sound.
5. **No asset pipeline.** All audio clips are code-synthesized (`synth_clip`;
   no import path), only 48 kHz f32 mono/stereo is accepted with no resampling
   and no compressed formats (`crates/matterweave-audio`), and detail geometry
   is authored cell-by-cell in code (`showcase.rs`, `flora.rs`, `fixtures.rs`).
6. **No editor and no settings UI.** There is no level/material editor surface
   and no settings seam (follows from 2 and 5); authors edit Rust constants and
   cell lists.
7. **Changed generator layouts abandon old journals instead of migrating.**
   `old_showcase_sessions_are_not_replayed_on_changed_layout`
   (`wetland_state.rs` tests) pins the behavior: old instance edits get a fresh
   recovery slot, never a migration. Correct conservatism, but a D4.2
   persistence-polish gap if layouts keep evolving.

## Insights

- The engine's hardest authoring rule is already enforced by types and tests
  (revision-gated uploads, publication deferral, atomic saves with caps,
  generator+seed rejection); the guide's main job was pointing authors at the
  existing enforcement sites rather than restating rules.
- The audio seam is the cleanest extension example in the tree (plain-data
  vocabulary, one owner, bounded queue, exhaustive-match compiler guidance for
  new variants) and the natural template for any future animation-event or
  settings plumbing — but that plumbing itself remains D4.2 work, not this
  slice.

## Validation

- `python3 tools/check_docs.py`: PASS — `213 Markdown files, 645 local links, 16 ADRs and 20 requirements.`
- `git diff --check`: PASS (no whitespace errors).
- Referenced identifiers/paths: validated by targeted `grep`/`sed` reads
  listed under Actions Taken before writing; no invented API.
- `git status` before commit showed only the two owned files modified;
  commit SHA recorded below. No push or PR per instructions.
