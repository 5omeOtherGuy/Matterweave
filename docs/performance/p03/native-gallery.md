# P03 native detail gallery (opt-in explorer viewer mode)

Status: working native viewer mode over the accepted P03 detail foundation. It is
an explicitly isolated fly/viewer mode, **not** native collision, not gameplay
integration and not the required dense showcase. Worker: native gallery A1
(bounded leaf). Session log: [logs/opus-native-gallery-a1.md](logs/opus-native-gallery-a1.md).

Initial worker scope: `apps/explorer/src/gallery.rs` (new), `apps/explorer/src/lib.rs`,
`apps/explorer/Cargo.toml`, the `Cargo.lock` local dependency edge, this document
and the session log. No renderer, physics, core, detail, shader or shared-board
files were changed by that worker.

## What this mode is

- Renders `matterweave_detail::gallery_scene(2026)` with the existing native
  Vulkan renderer, existing camera math and existing lighting defaults.
- `tile` and parasol presets use the sparse demonstration gallery: one16m tile
  and six ground-supported parasols. The additive `flora source` preset uses
  reviewed `dense_tile(20260908)`:84 plants/six types plus terrain. Neither fixture
  completes full-map density, native collision or the mandatory showcase.
- Flora uses Source LOD only. Both request parsing and direct construction reject
  Half/Quarter until their anatomy passes the separate quality gate. Generation
  runs once on entry; renderer recreation reuses the already combined CPU mesh.
- Owns no `World`, no `Physics`, no session and no save path. It cannot load,
  rename, replace or write the player's world, by construction.
- Reports truthful counters. Systems it does not run (physics steps, voxel
  bodies, chunk uploads, saves) report zero, never a borrowed gameplay number.

## Geometry adapter (explicitly labelled)

This is an **initial combined-mesh adapter**. It is **not GPU instancing** and
not the full-scale selected architecture.

1. `DetailScene::draws()` supplies instance/prototype/transform records.
2. One cached prototype `Mesh` per (prototype, LOD) is fetched with
   `DetailScene::prototype_mesh`. Prototype geometry is fetched once per
   prototype, never per instance; `SceneCounts::mesh_builds` proves this
   (2 builds for 7 instances at one LOD).
3. Each instance's cached vertices are transformed into world metres with the
   instance `Transform` (quarter-turn yaw, then metre translation) using the
   crate's own `point_to_world` / `direction_to_world`, and appended to one
   combined world-space `Mesh` reusing `matterweave_core::{Mesh, Vertex}`.
4. The combined mesh is uploaded **once per renderer** through the existing
   `Renderer::upload` whole-world compatibility path. There is no per-frame mesh
   rebuild and no per-frame upload; a renderer recreation re-uploads once.

Index arithmetic is validated, not assumed: prototype index counts must be a
multiple of three, every source index must lie inside its prototype's vertex
count, the running vertex base is `u32::try_from`-checked and
`checked_add`-checked, and the projected combined size is checked against
`MAX_COMBINED_MESH_BYTES` (64 MiB) before each append. No chunk coordinates are
fabricated as instance identifiers; the renderer's chunk map is not used at all.

Authoritative source data is untouched: source bytes, stored cells and expanded
cell counts are identical before and after combining (asserted in tests).

## Developer opt-in

The mode never starts implicitly. It requires one explicit request, resolved
**before any world is loaded**:

- `detail-gallery.txt` beside the normal `world.json` (app-private `files`
  directory on Android, the `--save` directory on the host), or
- the host environment variable `MATTERWEAVE_DETAIL_GALLERY`, which takes
  precedence when set to a non-blank value.

Request grammar, bounded to 128 bytes, `#` comments and blank lines ignored:

```text
<preset> [lod]
preset: tile | flora | parasol-front | parasol-side | parasol-underside
lod:    source (default) | half | quarter
```

Examples: `tile source`, `flora source`, `parasol-underside half`.
Coarse parasol views are explicitly experimental; they do not establish
close-range quality. Flora rejects coarse requests.

Viewpoints are derived from the actual scene bounds of the tile and the first
parasol instance (`bounds_world` under the instance transform), not from guessed
coordinates; yaw/pitch are computed to face the subject with the existing
`Camera::forward` convention. Preset selection requires a restart. There is no
new UI framework and no control remapping; normal-mode controls are unchanged.

Failure behaviour: an unreadable, oversized or unparsable request is a scoped
error that exits non-zero (host exit code 2) or aborts Android startup with a
logged error. It never falls through to the normal world and never writes user
data. There is no `unwrap`/panic on external marker content.

Opt-in `profile-frames.txt` frame captures work normally in this mode and write
the usual `frame-profile-v2-*.csv`.

## Reproduce (host)

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/target
cargo test -p matterweave-explorer --locked
cargo clippy -p matterweave-explorer --all-targets --locked -- -D warnings
cargo fmt -p matterweave-explorer --check
cargo build --locked -p matterweave-explorer --bin matterweave-explorer

dir=$(mktemp -d /mnt/bench/matterweave-dev/performance/gallery-smoke-XXXXXX)
printf 'sentinel user save bytes' > "$dir/world.json"     # user data sentinel
printf 'tile source\n' > "$dir/detail-gallery.txt"
timeout 120s xvfb-run -a "$CARGO_TARGET_DIR/debug/matterweave-explorer" \
  --smoke-frames 30 --gallery-exercise --save "$dir/world.json"
sha256sum "$dir/world.json"                                # unchanged
```

`--gallery-exercise` is a bounded, gallery-specific viewer check: it asserts a
single combined upload, requests and observes a real resize, recreates the host
window/renderer and asserts the recreation resets the one-time upload. It never
edits, saves or simulates, so it cannot be confused with `--smoke-exercise`,
which drives gameplay writes and is rejected in gallery mode.

## Executed checks

| Check | Result |
| --- | --- |
| `cargo test -p matterweave-explorer --locked` | PASS, 41 tests (29 pre-existing, 12 new). RED verified first against stubs: 9/9 new gallery tests failed. |
| `cargo test --workspace --locked` | PASS, 161 tests. |
| `cargo clippy -p matterweave-explorer --all-targets --locked -- -D warnings` | PASS (only the pre-existing vendored winit warning). |
| `cargo fmt -p matterweave-explorer --check` | PASS. |
| Host gallery smoke, `tile source`, Xvfb + llvmpipe | PASS, 30 presented frames, `--gallery-exercise` stages all passed, sentinel save unchanged, no files created. |
| Host gallery smoke, `parasol-underside half` / `parasol-side source` / `parasol-front quarter` (env opt-in) | PASS, 900/900/40 presented frames; sentinel unchanged. |
| Opt-in capture in gallery mode | PASS, 20-row `frame-profile-v2` CSV with zeroed physics/save/chunk counters. |
| Invalid request (unknown preset, 200-byte marker, `--gallery-exercise` without a request) | PASS, exit 2, no world load, sentinel byte-identical, no new files. |
| Normal-mode regression smoke (`--smoke-exercise --smoke-frames 30`) | PASS, edits/save/reload, grab/throw/fracture, resize and renderer recreation unchanged. |
| ARM64 debug APK build + `tools/verify_apk.py` | PASS, packaged `lib/arm64-v8a/libmatterweave_explorer.so`, 16 KiB LOAD alignment. Packaging only. |
| Device/phone execution, visual, lifecycle, memory and cost checks | NOT RUN by this worker (lead owns them). No install or device invocation was attempted. |

Host observations (Xvfb, llvmpipe software Vulkan, debug profile, opt-level 2).
These are software-rasteriser host numbers, not phone performance:

| Preset / LOD | Combined triangles | Combined mesh capacity | Prototype cache | Source payload |
| --- | --- | --- | --- | --- |
| `tile source` | 14,546 | 1,606 KiB | 691 KiB | 160 KiB |
| `parasol-underside half` | 4,180 | 467 KiB | 196 KiB | 160 KiB |
| `parasol-front quarter` | 1,590 | — | — | 160 KiB |

Counts are identical to the accepted source foundation: 2 prototypes,
7 instances, 26,113 unique stored cells, 30,803 expanded cells (30,204 collision,
599 liquid). Combined-mesh bytes and prototype-cache bytes are reported
separately and are vector capacities only: allocator metadata, map entries,
instance records, GPU buffers and renderer state are excluded. No
allocator-inclusive or process-memory claim is made.

Evidence (screenshots of the actual native renderer, run logs, capture CSV, APK
verification, sentinel hashes) is under
`/mnt/bench/matterweave-dev/performance/run-01/native-gallery-a1/`, with
`sha256-native-gallery-a1.txt` over the images and CSV.

## Lead integration after the initial worker

Flora source/evidence64661cf/267c2c8 merged through PR7 (`c71306d`), after
host/Android/docs CI passed. The existing source API was preserved; no duplicate
source audit was performed. App adapter test checks85 instances,28908 unique and
77810 expanded occupied cells and one mesh build perprototype. A malformed first
fixture compared `u64` with `usize`; corrected before the valid unknown-preset
RED (`f09111e`). Lead inspection additionally caught the inherited seed2026 HUD
label; generated scene and displayed provenance now share the tested preset seed.

Combined workspace:180 tests, all-target Clippy and formatting PASS in
`run-01/native-flora-workspace-green.log`. Explorer contributes45 tests, including
three P02 transactional upload-state checks. Normal-mode90-frame Vulkan validation
smoke passed edits/save/reload, grab/throw/fracture, resize and renderer recreation
in `run-01/p02-host-smoke/run.log`. Native flora render/phone checks and independent
combined-candidate reviews remain pending; initial worker evidence above does not
cover those later changes.

## Remaining gaps (not done here)

- No phone run: appearance, thermal, cost, peak memory and lifecycle on device
  are unverified. A successful APK build is not device evidence.
- No native collision, no gameplay integration, no detail data in saves.
- No GPU instancing, no per-instance culling, no streaming and no adaptive LOD;
  one combined draw payload per view.
- Full-map density, native flora appearance and the full showcase remain outstanding.
- Water is still opaque (no separate water pass), inherited from the foundation.
- The underside preset places the camera close to the supporting terrain, so
  terrain can occlude part of the frame; framing refinement is follow-up work.
- `docs/DEVELOPMENT.md` still lacks the gallery commands; the lead owns that file
  in this session.
