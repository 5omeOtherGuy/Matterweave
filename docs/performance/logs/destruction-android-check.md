# destruction-android-check — engineering log

Slice: a repeatable, rendered Android/host destruction diagnostic. It builds a
bounded welded source fixture, renders it through the real Vulkan renderer,
fractures it to exactly 64 unit pieces, simulates and renders the pieces,
round-trips the complete `PhysicsSave` state through JSON, and resets through 20
internal load cycles while checking body, collider and constraint counts, mass
and retained mesh bytes. Android physical execution is **NOT RUN** and is
lead-owned; the lead has a warm APK build and the phone.

Base `bf532eef15154936db730aa31544c53435601f57` (PR33 physics source already
present), branch `engine/destruction-android-check`. Source: the commit carrying
this log. Owned paths only: `apps/explorer/src/destruction_check.rs`,
`apps/explorer/src/lib.rs` (module, `--destruction-check`, Android marker value
`destruction`), this log.

## What changed

### Diagnostic ([apps/explorer/src/destruction_check.rs](../../../apps/explorer/src/destruction_check.rs))

- **Fixture (bounded).** 64x64 stone floor at y=0 (16 terrain chunk colliders)
  plus six bodies holding exactly 64 voxels: two welded 1 m cubes, one welded
  `[6, 2, 2]` beam (9 kg) resting/cantilevered on them, and three loose 1 m
  cubes. Source mass 24.000 kg, two persistent welds.
- **Phases (25, all drawn).** `source` -> `loose-fracture` (27 bodies, both
  welds preserved) -> `full-fracture-64` (exactly `MAX_BODIES` unit pieces, zero
  welds, 81 colliders) -> `serialize-reload` (JSON encode/decode equality,
  reload counts/poses, baseline weld structure restored through the same
  contract) -> `lifecycle-recreate` (zero extent Retry, renderer drop/recreate)
  -> 20 x `reset-cycle-NN`.
- **Per cycle.** Load the persisted baseline and check 6 bodies/23
  colliders/2 constraints; rebuild and upload the 6-box dynamic mesh; fracture
  to 64/81/0 and check unit-piece identity and finite conserved mass; simulate
  30 fixed steps; rebuild and upload the 64-box mesh; check the retained-byte
  bound and no growth after the first cycle. Fragmented frames are presented
  every cycle.
- **Real render path, not counters.** `DynamicMeshCache::update` ->
  `Renderer::upload_dynamic` -> `Renderer::render_with_lighting`; every phase
  requires `PHASE_FRAMES` (60 Android / 6 host) `FrameResult::Presented` frames.
  Actual mesh geometry is asserted at each phase: one box per live body,
  `24` vertices and `36` indices per box (6 boxes source, 27 loose, 64
  fractured), so a fake rebuild counter cannot pass.
- **Runtime invariants (checked in the executed phases, not only in tests).**
  - `check_unit_pieces`: exact `MAX_BODIES` count, every dimension `[1, 1, 1]`,
    every mass finite and positive, summed mass within `1e-4` kg of 24.000 kg.
  - `check_retained_bound` + cycle stability: `DynamicMeshCache::retained_bytes`
    <= declared `MESH_RETAINED_BOUND_BYTES` (137216; 2x the 64-box payload plus
    8 KiB scratch) and identical after every cycle following the first.
  - `check_finite`: every saved position/velocity/rotation finite after
    simulation.
  - `check_welds_match_baseline`: live weld topology equals the persisted
    baseline (loose fracture) and restores exactly (serialize, cycles).
- **Diagnostic-only HUD.** Every drawn frame carries three short
  `Hud::text` lines built from the engine's existing immediate HUD API (no new
  UI framework, no gameplay change): `phase N/24 <name>`, `bodies B welds W
  colliders C` (read from the physics state behind that frame), and `drawn D
  epoch E`. This is fixed by a unit test on `diagnostic_lines`.
- **Lifecycle.** `suspended` drops the renderer/window and counts a platform
  event; `resumed` recreates it and restarts the current phase from the saved
  baseline. Internal renderer recreation in the `lifecycle-recreate` phase is
  counted separately (`internal_recreations`) and labelled as not an Android
  HOME/RESUME event. Zero-extent frames return `Retry` and are never counted.
- **Failure handling.** Any invariant error records `FAIL destruction: ...`,
  stops redraws and exits (`el.exit()`), and the host runner exits non-zero.
  A `PASS destruction: ...` line is written only after all 25 phases have
  presented their frames and the final reset structure was drawn once.
- **Isolation.** Owns no save path: it never reads or writes a world save; the
  only file written is `destruction-check-report.txt` beside the caller path.
  Android consumes `files/engine-check.txt` value `destruction` once, like the
  existing diagnostics, before the chooser runs.

## Verification executed (host, frozen source)

Raw logs and the final report are preserved in
`/mnt/bench/matterweave-dev/coarse-terrain/destruction-android-worker/evidence/`.
No performance, thermal, power or device claim is made.

| Check | Command | Result |
| --- | --- | --- |
| Explorer suite | `CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/coarse-terrain/target-destruction-check CARGO_BUILD_JOBS=2 cargo test -p matterweave-explorer --locked` | PASS: 141 passed, 1 pre-existing ignored, 0 failed |
| Workspace suite | `... cargo test --workspace --locked` | PASS: exit 0, 574 passed, 3 pre-existing ignored, 0 failed |
| Format | `cargo fmt --all -- --check` | PASS |
| Scoped strict Clippy | `... cargo clippy -p matterweave-explorer --all-targets --locked -- -D warnings` | PASS: 0 errors; only the pre-existing vendored-winit function-cast warning remains (capped by Cargo) |
| Docs | `python3 tools/check_docs.py` | PASS: 204 Markdown files, 623 local links, 16 ADRs, 20 requirements |
| Host Vulkan smoke | `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json timeout 300s xvfb-run -a cargo run --locked -p matterweave-explorer -- --destruction-check --save /tmp/destruction-unused.json` | PASS: app exit 0, 25/25 phases with 6 presented frames each (151 total), 0 Vulkan validation errors, one `PASS destruction` line, no save file created. Renderer: llvmpipe (LLVM 20.1.2), Vulkan 1.4.318, validation on. |
| FAIL path mutation: wrong loose-fracture count | `LOOSE_FRACTURE_BODIES + 1` (reverted) | PASS (gate discriminates): app exit 1, `FAIL destruction: loose-fracture: bodies/constraints/colliders (27, 2, 44), expected (28, 2, 45)`, no PASS line |
| FAIL path mutation: runtime mass check | doubled summed mass inside `check_unit_pieces` (reverted) | PASS (gate discriminates): app exit 1, `FAIL destruction: full-fracture: fractured mass 48 kg, expected source 24 kg` |
| FAIL path mutation: runtime retained bound | `MESH_RETAINED_BOUND_BYTES = 8 * 1024` (reverted) | PASS (gate discriminates): app exit 1, `FAIL destruction: full-fracture: retained dynamic mesh bytes 90384 exceed the declared bound 8192` |

All three mutations were reverted from a pristine copy and verified byte
identical (`cmp`), with `cargo fmt --all -- --check` clean afterwards.

### Measured report values (host llvmpipe, deterministic CPU state)

- Source: `bodies=6 welds=2 colliders=23 voxels=64 mass_kg=24.000 mesh_boxes=6`.
- Loose fracture: `bodies=27 welds=2 colliders=44 mesh_boxes=27`.
- Full fracture: `bodies=64 welds=0 colliders=81 unit_pieces=true
  mass_kg=24.000 retained_bytes=90384 mesh_boxes=64` after 30 simulated steps.
- Serialization: `json_bytes=16204 decoded_equal=true reloaded_bodies=64
  reloaded_welds=0 reloaded_colliders=81 baseline_bodies=6 baseline_welds=2`;
  body poses/velocities/rotations reload within `1e-5`.
- Cycles: every one of the 20 reset cycles returned to `6/23/2` and re-fractured
  to `64/81/0 mass_kg=24.000`; `retained_bytes=92160` identical from cycle 0
  onward, below the 137216 declared bound. PASS totals: `total_presented=151
  uploads=170 rebuilds=169 internal_renderer_recreations=1 platform_suspends=0
  platform_resumes=0 zero_extent=Retry`.

### Host exit-status note

One early wrapper invocation returned shell status 2 while its report said PASS.
That status came from a trailing `ls /tmp/destruction-unused.json` used to prove
the save file was never created: the file was correctly absent, so `ls` failed.
The app process itself exited 0 in the clean rerun (`APP_EXIT=0`, direct capture,
raw log `destruction-final.log`). There is no case of a failed host process
reporting PASS.

### Mutation-method note

One `sed`-based mutation attempt failed to apply (unescaped delimiter), so it ran
the unmutated binary and its captured report was stale. It was discarded; the
mutation results above were produced with the `edit` tool and each reverted
state was verified with `cmp`.

## Android status

- **Physical Android execution: NOT RUN** (lead-owned; warm APK build and phone
  are with the lead).
- The `aarch64-linux-android` cross-build in the separate target dir
  `target-destruction-check-android` was **interrupted by the lead**
  (PID 2225043, duplicate cold build) and is **not complete and not counted**.
  No NDK/toolchain setup was performed, and no repository or lockfile change
  resulted. Per lead instruction it was not restarted; host checks are the
  frozen handoff evidence.
- Android lifecycle correctness therefore rests on source review plus the host
  zero-extent and internal-recreation phases. Real HOME/RESUME is exercised on
  the phone by the lead; the report distinguishes platform suspend/resume
  events from the 20 internal reset/load cycles, and declares zero platform
  events on the host run above.

## Non-claims

- No FPS, energy, thermal or sustained-workload claim; this is functional
  correctness only.
- The host render used llvmpipe software Vulkan. It is not an Android driver
  result, and no mobile performance may be inferred.
- The 64-piece fixture is a bounded diagnostic, not a stress or soak workload.
- This does not itself modify or integrate the physics/renderer crates; it uses
  the merged PR33 surface (`break_body`, `constrain`, `save`/`load`,
  `PhysicsSave`, `body_mass`, `constraint_count`, `collider_count`) unchanged.

## Developer invocation

Host:

```sh
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/coarse-terrain/target-destruction-check
CARGO_BUILD_JOBS=2 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  timeout 300s xvfb-run -a cargo run --locked -p matterweave-explorer -- \
  --destruction-check --save /tmp/unused.json
grep '^PASS destruction' /tmp/destruction-check-report.txt
```

Android (device lead; one-shot marker consumed before the chooser):

```sh
adb shell run-as dev.matterweave.explorer \
  sh -c 'echo destruction > files/engine-check.txt'
# launch the activity; the report is written to files/destruction-check-report.txt
```

## Handoff / remaining risk

- Preserved evidence: `destruction-final.log`, `destruction-check-report.txt`,
  `workspace-tests.log`, `explorer-tests.log`, the clean rerun log, and the
  three FAIL-path logs under
  `/mnt/bench/matterweave-dev/coarse-terrain/destruction-android-worker/evidence/`.
  Final report SHA-256 `3c978e8367ba32193da57b1b500ddef14d49aa3149b1b91ba61ea8a9bc2544d3`;
  source file SHA-256 `a89a51c727416af069d9110c31a3c47f45f68552d1162271210c66540e13fc80`.
- Lead-owned next: independent review, actual phone run (APK + marker +
  screenshots; optionally HOME/RESUME mid-sequence and read the report lines),
  and integration into the delivery checkout. M2-M6 gates are unaffected by
  this diagnostic; it closes no engine milestone.


## Lead review and repair — 2026-09-12

- Actions taken: integrated diagnostic as `d22954c`; ran physical Android with
  1501 presentations, 64 pieces, 20 reset/load cycles and one real HOME/resume.
- Issues & friction: independent Gemini found repeat preparation on resize and
  final-frame Retry re-entry. The first physical PASS did not discriminate these
  boundaries; it is not final corrected-source acceptance.
- Decisions & rationale: preserve authoritative simulation across renderer loss;
  keep finalization independent from completed-phase reporting. Retained-byte
  evidence covers DynamicMeshCache only, not general memory-leak freedom.
- Solutions applied: lead `5347e91` adds the real progression seam and regressions
  for surface loss, skipped presentations and repeated final Retry. Corrected
  Android run is pending while the owner has the grove open.
- Insights: a successful lifecycle example is insufficient when callbacks can
  arrive at other phase boundaries. Preserve first-run evidence and test those
  boundaries directly.

Corrective acceptance: `5347e91` passes 157 app tests (1 existing ignored),
scoped strict Clippy and 151 host Vulkan frames. Repaired Android run passes
1501 presentations, 25 unique 60-frame phase summaries and HOME/resume during
phase 4; current four save hashes unchanged. Gemini `w_004fc6f2` found no defects.
The original worker is accepted after lead repair, not first-pass accepted.
