# Matterweave authoring and extension guide

A practical guide to building game content on the **current** engine, grounded in
the actual crate interfaces and the two shipped samples. Every signature below
names its real source path; anything not executable as written is labeled
pseudocode. This guide covers documentation scope only — it does not close D4
functional acceptance or any device gate.

Setup, host build/test, Android build and gates live in
[DEVELOPMENT.md](DEVELOPMENT.md) and are not duplicated here.

## What is reusable vs what is sample glue

**Reusable public crates** (depend on these from new game code):

| Crate | Owns | Key source |
| --- | --- | --- |
| `matterweave-core` | Authoritative coarse voxels, persistence, input, reference queries | `crates/matterweave-core/src/lib.rs` |
| `matterweave-detail` | Fine multi-resolution prototypes, scenes, materials | `crates/matterweave-detail/src/lib.rs`, `scene.rs` |
| `matterweave-physics` | Characters, rigid bodies, collision derived from worlds | `crates/matterweave-physics/src/lib.rs` |
| `matterweave-render` | Vulkan raster renderer, HUD, lighting/reflection upload | `crates/matterweave-render/src/lib.rs` |
| `matterweave-audio` | Deterministic mixer service, clip/voice API | `crates/matterweave-audio/src/service.rs` |
| `matterweave-pacing` | Deterministic frame-loop pacer | `crates/matterweave-pacing` |

**Explorer-internal glue** (do not import; copy the pattern instead):
`apps/explorer/src/audio_service.rs` (`AudioAdapter`, `GameplayEvent`,
`EventQueue`, `AudioScope`), `experience.rs` (`Experience`, the one audio
owner), `controls.rs`, `wetland.rs`, `voxel_relay.rs`, `wetland_state.rs`,
`terrain_lab.rs`, `gallery.rs`, and the CLI wiring in `lib.rs`
(`run_desktop`). There is no plugin system or stable ABI: samples compile into
the explorer binary.

## Adding a voxel prototype and material

Fine detail lives in `matterweave-detail`. A prototype is a `DetailVolume` at a
fixed physical scale, placed into a `DetailScene` as named instances.

```rust
use matterweave_detail::{material, DetailScene, DetailVolume, Scale, Transform, Yaw};

// Prototype: authored source cells. `set` returns `Result<bool>`; budgets are
// enforced before allocation (`MAX_VOLUME_CHUNKS`).
// Source: crates/matterweave-detail/src/lib.rs (`DetailVolume::set`).
let mut stone = DetailVolume::new("my-stone", Scale::new(0.125)?);
stone.set([-2, 0, 0], material::BANK_STONE)?;

// Scene: register the prototype, then place instances by id.
// Source: crates/matterweave-detail/src/scene.rs (`add_prototype`, `place`).
let mut scene = DetailScene::new();
scene.add_prototype(stone)?;
scene.place("stone-1", "my-stone", Transform::new([-2., 0., -2.], Yaw::Deg90)?)?;
```

Materials are plain `u8` ids with a fixed policy table, not a material graph:

- `material::AIR` (0) is empty. Core-world ids 1–8 are the island/chamber set
  (grass, dirt, stone, sand, trunk, leaf, mineral, crate-adjacent ids — see
  `World::generate` and `generate_chamber` for the actual mapping your scene
  uses). Detail ids include `DETAIL_SOIL` (10), `MOSS_TURF` (11),
  `BANK_STONE` (12), `WATER` (13), mushroom 20–23 and flora 30+ series.
- `matterweave_detail::material_policy(id)` decides behavior
  (`MaterialPolicy::Collision` participates in gameplay/collision; decorative
  leaves and liquid do not mask editable solids in `wetland_state::raycast`).
- `material_name` / `material_color` are display helpers.
- Source: `crates/matterweave-detail/src/lib.rs` (`material` module,
  `material_policy`).

There is **no asset pipeline**: no model/texture import, no compressed audio,
no material editor. Geometry is authored in code cell by cell (see
`crates/matterweave-detail/src/showcase.rs`, `flora.rs`, `fixtures.rs` for the
real authoring patterns). There is **no animation system**; motion comes from
physics bodies and placement transforms only (see "Missing features" below).

## Authoritative vs derived data

The rule the engine enforces everywhere: **world data is authoritative; meshes,
collision and lighting are derived and versioned, and stale derived results must
never overwrite newer sources.**

- **Authoritative:** `World` cell materials (`get`/`set`, `revision()`),
  `DetailScene` prototype sources and instance placements
  (`source_version()`, per-prototype `revision()`). Edits go here first.
- **Derived, revision-checked:** `Mesh.revision` carries the *source* revision
  it was built from; `Renderer::chunk_revision` / `World::chunk_revision`
  gate uploads (`apps/explorer/src/lib.rs`, `sync_render_meshes` only uploads
  when revisions differ); `DetailScene::prototype_mesh(id, lod)` caches per
  source revision; coarse tiles (`matterweave-core/src/coarse.rs`) are
  read-only derivations with source revision/mode provenance.
- **Collision publication:** `Physics::sync_world(&world)` (core worlds) and
  `Physics::replace_detail_scene(&scene)` (detail scenes) rebuild colliders;
  the wetland frame path additionally defers newly added solid material while a
  body overlaps the edited cell (`apps/explorer/src/wetland.rs`, `edit()`).
  Removals publish as soon as preparation completes.

Consequences for authors: after `edit_instance` / `World::set`, re-realize the
edited prototype (`prototype_mesh(&id, Lod::Source)`), refresh only the
affected renderer uploads, and never cache a mesh or collider without its
source revision. The wetland `edit()` in `apps/explorer/src/wetland.rs` is the
reference implementation of this sequence, including rollback when the derived
budget cannot serve the edit.

## Composing an instance / scene

Follow the wetland pattern (`apps/explorer/src/wetland.rs`, `Runtime::load`):

1. Build the scene: `matterweave_detail::build_showcase(SHOWCASE_SEED)?` for
   the full wetland, or assemble `DetailScene` from your own prototypes as
   above. `build_showcase` returns a `Showcase` container, not a scene — take
   its `scene` field:

   ```rust
   let showcase = matterweave_detail::build_showcase(matterweave_detail::SHOWCASE_SEED)?;
   let scene = showcase.scene;
   ```

2. Publish collision once: `physics.replace_detail_scene(&scene)?`.
3. Per frame: poll background preparation, publish on the frame cadence
   (`sync_detail_collision` pattern), upload only changed prototype meshes at
   the LODs actually drawn (`select_lods` / `prepare_batches` —
   `crates/matterweave-detail/src/scene.rs`), and drive the static renderer
   path (`Renderer::replace_static_scene` / `update_static_instances`).
4. Ray-query in source space for interaction:
   `wetland_state::raycast(&scene, origin, direction, distance)` returns the
   hit `instance`, `cell`, an exposed-face `previous` cell for placement, and
   `distance`. It skips decorative overlap and rejects non-finite input.

Coarse core worlds compose differently: `World::generate(seed)` (island
fixture) or cell-by-cell `World::set`, `enable_streaming()` +
`stream_around(eye)` for residency, `world.mesh_chunk(key)` /
`renderer.upload_chunk(key, &mesh)` for graphics, `physics.sync_world(&world)`
for collision. The Relay chamber (`generate_chamber` in
`apps/explorer/src/voxel_relay.rs`) is the complete small example: floor,
walls, dividing wall, door and obstacle cells from explicit cell lists
(`DOOR_CELLS`, `OBSTACLE_CELLS`), with gameplay constants
(`CHAMBER_SEED = 20260909`, `CRATE_DIMENSIONS = [2, 2, 2]`, `EXIT_Z_MIN`).

Shared renderer services both samples use: `Renderer::render` /
`render_with_lighting` with `LightingSettings { sun: Sun { direction_to_sun,
intensity }, shadows, shadow_map_size }`, `Hud::new(w, h)` plus
`hud.rect(...)` / `hud.text(...)`, dynamic bodies via `DynamicMeshCache` +
`Renderer::upload_dynamic`, opt-in `upload_indirect` / `upload_reflection`
with matching `disable_*` calls.

## Connecting game input, actions and audio

One vocabulary, one owner. `matterweave_core::InputService`
(`crates/matterweave-core/src/input.rs`) is the shared, platform-independent
tracker: `set_move_zone(rect, radius)`, `add_button_zone(rect, action_id)`,
`add_motion_zone(rect, axis, latch)`, `pointer_down/move/up`,
`key_down/key_up(VirtualKey)`, and per-frame `consume_motion()` /
`consume_look()` / `take_action(id)`. Always `input.clear()` on
resume/suspend/focus-loss so no pointer or key sticks
(`voxel_relay.rs`, `setup_input` + `resumed`/`suspended`, is the minimal
example).

Gameplay facts become sound through `GameplayEvent`
(`apps/explorer/src/audio_service.rs`): `Footstep`, `Jump`,
`Land { strength }`, `BlockEdit`, `Objective`, `UiConfirm` — the whole
vocabulary, plain data with no platform type. Samples push into a bounded
`EventQueue` (`push` drops oldest at capacity 16; `pop` drains; `clear` drops
all on pause/switch):

```rust
// In sample logic (source: apps/explorer/src/voxel_relay.rs):
self.events.push(GameplayEvent::Objective);

// Owned by the app, drained once per frame (source: experience.rs, pump_audio).
// A sample run standalone (e.g. `--voxel-relay`) has no audio owner and is
// silent by construction: its queue is simply never drained.
```

Direct service use (inside adapters, not gameplay): `AudioService::new()`,
`register_clip(ClipSpec::mono(&samples))`, `play(clip,
PlayOptions::with_gain(g))`, `stop_voice`, `set_voice_gain`, per-frame
`poll_device()`, lifecycle `suspend()` / `resume()`. Limits are hard:
32 clips, 8 voices, 4 MiB PCM, 64 commands; refusal is an explicit
`AudioServiceError`, never blocking. Only 48 kHz f32 mono/stereo is accepted;
there is no resampling and no compressed format support.

Physics actions available to gameplay: `physics.step(dt, velocity, jump)` /
`step_objects(dt)` for characters and bodies, `teleport(eye)` and
`set_flying_eye(eye)`, `grab(&world, origin, dir, range)`, `throw(dir)`,
`break_body(&world, origin, dir, range)`, `spawn_playground(&world)`,
`snapshot()` / `restore(&snapshot)`. Edits must call `sync_world` (or
`replace_detail_scene`) afterwards or gameplay and visuals diverge.

## Persistence and seeded reproducibility

Two proven patterns; pick by world type:

- **Core worlds (sandbox, Relay):** `World::save(path)` /
  `World::save_with_attachment(path, Some(json))` /
  `World::load(path)` (`crates/matterweave-core/src/persistence.rs`).
  Atomic same-directory temp + rename; the previous file survives failed
  writes. Caps: 512 chunk overrides, 12 MiB total — edits beyond the cap are
  refused, existing data stays editable. `FORMAT_VERSION = 2`,
  `GENERATOR_VERSION = 1`; mismatched versions fail the load. Relay stores
  `VoxelRelaySave { version: 1, physics, door_open, obstacle_cleared, solved,
  status }` as the attachment, keeping voxels and puzzle state in one file
  (`VoxelRelayApp::save` / `load`).
- **Wetland:** `SavedWetland { version: 1, generator, seed, edits,
  physics, yaw, pitch, shadows }` (`apps/explorer/src/wetland_state.rs`) —
  a bounded edit journal (4096 edits, 2 MiB), replayed onto the pinned
  generator. Recovery never overwrites: corrupt saves are retained
  byte-for-byte and numbered `recovery-N` slots are tried newest-first via
  `load_recovering_with`, which accepts a candidate only after `restore`
  actually succeeds against the live scene and physics.

Seeded reproducibility: generation is deterministic integer math on the seed —
`World::generate(seed)`, `generate_chamber(seed)` with `CHAMBER_SEED`, and
`build_showcase(SHOWCASE_SEED)` with `SHOWCASE_GENERATOR_VERSION`. Record the
`(generator, seed)` pair with every save (both patterns do) and reject unknown
pairs on load rather than replaying stale edits onto a changed layout (see the
`old_showcase_sessions_are_not_replayed_on_changed_layout` test).

Desktop entry points for iteration: `--save PATH` (default
`matterweave-world.json`), `--showcase` for the wetland, `--voxel-relay` for
Relay, `--terrain-lab` for the detail lab; `--smoke-exercise` with a fresh
`--save` path exercises real place/remove, grab/throw/fracture, save/reload,
resize and renderer recreation on host (source: `run_desktop` in
`apps/explorer/src/lib.rs`). Full flag and gate list: [DEVELOPMENT.md](DEVELOPMENT.md).

## Registering a new native sample

Samples are compiled into the explorer binary; there is no dynamic loading.
Concretely, adding e.g. a `SlalomApp`:

1. **Create `apps/explorer/src/slalom.rs`** with the sample-owned struct:
   `world: World` (or `scene: DetailScene`), `physics: Physics`,
   `input: InputService`, `renderer/window`, `save_path`, `events:
   EventQueue`, plus `new(save_path, frame_limit)`, `setup_input()`,
   `update(dt)`, `save()` via `World::save_with_attachment`, `load()` via
   `World::load` plus `world.attachment()` (see `VoxelRelayApp::save`/`load`
   in `voxel_relay.rs`), and `pop_event()` / `clear_events()` accessors. Reuse
   `generate_*` + `initial_physics` style constructors with a pinned seed
   constant.
2. **Add an `AudioScope` variant** in `audio_service.rs`
   (`Wetland | Sandbox | VoxelRelay` today) so scope switches retire the
   leaving sample's voices (`switch_to`).
3. **Wire `Experience`** (`experience.rs`): store the app, drain its queue in
   `pump_audio`, retire scopes in `switch_if_requested`, forward
   `resumed`/`suspended`/`window_event` (clearing input and events on
   pause/focus-loss exactly like the existing samples).
4. **Add entry points in two places.** `lib.rs` owns only the standalone flag:
   a `--slalom` branch in `run_desktop` (silent audio by construction, like
   Relay). The chooser path is *not* in `lib.rs`: give the sample a
   `for_chooser(legacy_path, frame_limit)` constructor mirroring
   `VoxelRelayApp::for_chooser` (`voxel_relay.rs`) with a dedicated
   `<name>.json` save file that never writes back to the legacy path, call it
   from `Experience::switch_if_requested` (`experience.rs`), and set the
   transition flag from the chooser button handler in `wetland.rs` (today:
   `sandbox_requested`, `voxel_relay_requested`, `terrain_lab_requested`).
5. **Verify without hardware:** `cargo test -p matterweave-explorer`, then the
   `--slalom` flag you added:
   `cargo run -p matterweave-explorer -- --slalom --save /tmp/<fresh>.json`;
   re-run `python3 tools/check_docs.py` if docs change.

Both shipped samples demonstrate the no-fork rule: the wetland (detail scene,
first-person, edit journal) and Relay (core `World` chamber, orthographic
`camera_view_proj`, attachment save) share `InputService`, `AudioAdapter`,
`Physics`, `Renderer` and the persistence helpers with zero engine duplication.
A new sample must do the same — any logic two samples need belongs in a crate
or in the shared adapter, not copied into the sample. Sample reuse is the part
of the D4.2 gate that is demonstrably done; the physical-input and audibility
observations are not (see below).

## Missing features — do not assume these

Engine capabilities that genuinely do not exist yet:

- **No animation system.** Neither shipped sample needs animated geometry
  today; ROADMAP M6's "minimal animation … those samples actually need"
  resolves to zero for the wetland and Relay, whose motion is physics bodies
  and placement transforms. A future sample that needs animation is new engine
  scope, not a configuration of an existing system.
- **No asset pipeline or importers.** No model/texture/audio import and no
  compressed audio: detail geometry is authored in code
  (`crates/matterweave-detail/src/{showcase,flora,fixtures}.rs`) and audio
  clips are synthesized in code (`synth_clip`).
- **No level/material editor and no settings UI.** Authors edit Rust constants
  and cell lists. Mute/volume (`AudioAdapter::set_muted` / `set_volume`) exist
  in `audio_service.rs` but no sample UI drives them.
- **No stable ABI or dynamic samples.** `Experience` wiring and
  `AudioScope` are compiled-in; third-party out-of-tree samples are not
  supported.

Scope boundaries and open acceptance — do not report these as shipped:

- **Unmerged worker work is not shipped.** Shared mobile settings and
  additional lighting-response work are in flight in unmerged worker branches;
  do not treat settings services, GI/reflection response, or their APIs beyond
  the surface documented above as available until they land in `main`. The
  renderer surface documented in this guide is what `main` ships.
- **D4.2 physical acceptance is still open.** Existing sample reuse is real:
  both samples run without engine forks on the shared crates, and functional
  input plus persistence are demonstrated by tests and the Relay device gate.
  Physical simultaneous multi-finger use, lock/unlock and audible output remain
  unverified — STATUS records sequential ADB gestures, concurrent touch covered
  only by unit tests, and no human audibility confirmation. These are device
  acceptance items, not engine API gaps.
- **Host audio proves nothing about devices.** The mock backend renders into
  a buffer for tests; host runs are silent, standalone sample runs are silent
  by construction, and counters are not audibility.
- **Derived-data limits still apply:** reflection volume is 64³ unit-voxel
  with one secondary ray; indirect lighting reference coverage, coarse
  transitions and sustained-efficiency acceptance remain open per
  [STATUS.md](STATUS.md). Do not promise them to players.
