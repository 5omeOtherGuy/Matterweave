# Native automatic detail check

## What problem this solves

The automatic LOD selection in
[automatic detail engine](automatic-detail-engine.md) had no end-to-end check
that connected selection to retained Vulkan prototype geometry and instance-only
updates.

## How it works

The engine check connects `DetailScene::prepare_batches` to the Vulkan renderer's
retained prototype geometry and instance-only updates. Two source volumes (an
irregular solid and a one-cell sheet) supply four instances, including negative
coordinates and quarter-turn placements. Six `Source`/`Half`/`Quarter` meshes are
preloaded. Ordinary camera phases update instance records; source edits and
renderer recreation replace the resident catalog. A source token rejects stale
catalogs, and every selected mesh revision is checked against authoritative data.

Nine phases cover approach/retreat, perspective FOV zoom, orthographic zoom, an
edit, a resident zero-build budget, zero-size surface Retry and renderer
recreation. The sheet stays at `Source`. A source collision/sample probe remains
unchanged by camera selection. This is a query invariant, not a full Rapier
traversal test. Cold-cache budget fallback is tested on the host; the rendered
budget phase has all coarse meshes resident and therefore proves only zero
additional mesh builds.

The adapter is opt-in engine validation: no framework or showcase content was
added. It reuses the existing LOD and Vulkan instance APIs and retains source
query authority independently of camera choice.

## What was verified

Host and Android.

- `cargo test --locked -p matterweave-explorer detail_check`: 8 tests passed
  after integration at `f33c488`. The stale-catalog test failed at the RED
  checkpoint `fb7ddad` (integrated as `5c95668`) before the source-token fix;
  an initial 8-phase native run passed, and the perspective FOV phase was added
  after that checkpoint.
- The integrated 54-frame native check passes all 9 phases and selects
  `Source`/`Half`/`Quarter` under llvmpipe LLVM 20.1.2 Vulkan 1.4.318 with
  validation enabled; no validation errors. Raw report/log:
  `/mnt/bench/matterweave-dev/performance/engine-02/detail-native-integrated`.

```sh
mkdir -p /mnt/bench/matterweave-dev/performance/detail-check
MATTERWEAVE_VALIDATION=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  xvfb-run -a cargo run --locked -p matterweave-explorer -- --detail-check \
  --save /mnt/bench/matterweave-dev/performance/detail-check/unused.json
```

Require terminal `PASS detail` in `detail-check-report.txt` and no Vulkan errors.
The command uses only the save argument's parent for its report; it loads no
world.

For Android, write `detail` to the app's `files/engine-check.txt`, then launch.
The one-shot request is consumed before ordinary entry; the report is written to
`files/detail-check-report.txt`. Android holds each phase for 120 presented
frames. Delete an earlier owned report before capture to avoid attributing stale
phase markers to a new run.

The OnePlus 13 Android 16 run passes 1080 presented frames across all 9 phases,
including real HOME/resume in phase 3 and the explicit recreation in phase 8.
Three capability reports identify the recreated renderers. All 9 phase
screenshots were captured; lead viewed 0/3/5. `Source`/`Half`/`Quarter`
selections and unchanged thin sheet/source query are recorded. No validation
layer is installed on the phone. See
[device manifest](../evidence/2026-09-08-detail-engine.json) and
[actual report](../evidence/2026-09-08-detail-engine-report.txt).

A scoped Clippy run was interrupted during dependency checking and is not
counted as passed. An initial budget-fallback label overstated what preloaded
geometry proves; it was corrected.

## Limits and open work

- Detail error is heuristic and global dilation cannot preserve every small
  cavity. Abrupt transitions, visual stability, mobile cost and production
  residency policy remain open.
- Phone images are low contrast; they establish presentation of the fixture, not
  final material/lighting or transition quality. Native execution is not mobile
  performance evidence.
- Fixture drift: the measurements above use the two-prototype, four-instance
  fixture at `f33c488`. `apps/explorer/src/detail_check.rs` has since gained a
  third prototype and a fifth instance.
