# Current status

## Post-reboot primary-plan development — 2026-09-12

The owner prioritizes development batches over repeated intermediate live checks.
Host tests and independent review remain required; Android acceptance is batched
on integrated behavioral changes. Two free Muse workers implement native
mesh-lighting publication and the actual continuous-edit streaming progress fix.
Gemini corrective review of detail05549eb and streamingf63055b found no concrete
defects. Streaming host contract checks are in PR37; application progress and
Android streaming acceptance remain open.

Clean CI source4a60df7 (tree identical to mainac1ce444) was re-signed with the
existing debug key, verified for signature/16KiB alignment, installed without data
clear, and visually ran both wetland and the existing solved Relay save. All four
original saves were verified restored byte-for-byte after smoke. This revalidates
the existing install/run slice, not the final E8 candidate.
[Manifest](evidence/2026-09-12-restart-smoke/manifest.json).


**Restart checkpoint:** all workers finished; source commits and outstanding gates are listed in [restart handoff](performance/restart-20260912.md). This checkpoint supersedes older live-worker descriptions below.

## Recovery and continued development — 2026-09-12

PR #35 merged at `ac1ce444`: reviewed, host/device-verified destruction diagnostic,
all six CI checks green. Audio audibility is confirmed by the owner. A later
`/mnt/bench` hardware I/O failure interrupted follow-up development, not that delivery.
Source work continues in isolated healthy-disk recovery checkouts; benchmark work
and data remain deferred. [Recovery state](performance/recovery-20260912.md).

DeepSeek Go returned HTTP429 weekly quota errors. Per owner direction, the live
D1 repair worker is OpenRouter `deepseek/deepseek-v4.1-flash` high (`w_f38feb0e`);
D3.2 recovery uses exact Muse Spark 1.3 Contributor Free high (`w_a0a724e3`).
Both have executed source-reading tool calls. Gemini `w_3287036b` reviewed frozen
D2.3 Git objects after its earlier result became unreadable. OpenRouter DeepSeek
`w_b5255700` now repairs its concrete bound-scope and byte-admission test gaps;
sustained-edit app liveness remains open. No Opus or Astra worker was launched. No top-up or overage enabled.


Terrain Lab (PR #29) and production wetland detail integration (PR #31) are merged and functionally verified on Android. Choose **EXPLORE TERRAIN LAB** for the playable terrain comparison. [Controls and evidence](performance/terrain-lab.md). Shared audio PR #32 merged at `f370990` after repaired Android resume checks and all six CI checks; owner hearing is now confirmed.

Updated: 2026-09-12. Latest published prerelease: **v0.5.0**. M2–M6 remain open.

## Playable lab and production integration — 2026-09-12

PR #29 merged at `bd01a65`, all six checks passing. Terrain Lab is installed and
functionally verified on OnePlus13; see [controls and evidence](performance/terrain-lab.md).
PR #31 wires the production wetland camera to the detail runtime and passes the
[Android render/edit/persistence/lifecycle slice](performance/wetland-auto-detail.md).
Actual tested wetland views stayed at Source under retained quality guards;
coarse transition acceptance remains open. Shared audio at `b2ffd5b` passes147 app tests,49 audio tests, strict scoped
Clippy/fmt and independent Gemini review. The phone exposed AAudio resume failure
-899; after repair, **3/3 wetland cycles and one Relay cycle** reproduced the
failure and recovered automatically to successful stream starts and real events.
[Shared audio evidence](performance/shared-sample-audio.md). PR #32 merged at `f370990`, all six checks passing; owner hearing confirmation is recorded below.
All original user saves and the completed Relay import were restored and hashed.

Current reconciliation supersedes the out-of-order coordination messages:
PR #33 physics merged at `c86980b`; PR #34 production mesh lighting merged at
`16ad342`. No open PRs at this observation. Claude remains an engine-slice
producer; Codex owns app integration, acceptance and the phone.

The destruction diagnostic at `d22954c` passed its first Android run: 1501
presentations, exactly 64 pieces, 24 kg conserved mass, 20 internal reset/load
cycles, one internal renderer recreation and one real HOME/resume. Independent
Gemini review then found duplicate phase execution on resize and incorrect final
phase reporting after a presentation Retry. Lead correction `5347e91` passes 157 app tests, scoped Clippy,
151 host Vulkan frames and a repeated Android run with HOME/resume specifically
during phase 4: 25 unique 60-frame phases, one internal recreation, four current
user-save hashes unchanged. Corrective independent Gemini review `w_004fc6f2` passes. Retained-byte evidence covers the
dynamic mesh cache only. [Detailed evidence](performance/destruction-android.md).

D1.2 production detail fixtures are committed at `d8a5cde`; Gemini worker
`w_1f749699` found two real diagnostic gaps; DeepSeek `w_761c1314`
resumes the original session to repair surface-callback handling and validate
packed renderer transforms. D2.3 continues in the preserved exact
session (`w_5484fb7e`); contributor D3.2 runs as `w_7ea6fc6d`. The three-worker
cap includes the reviewer. No duplicate assignments or Astra workers.

The owner subsequently confirmed: "Audio works 100% it is all yours." Human
audibility is accepted on that report and the phone is released for Android tests. The app exit coincided with lead
diagnostic deployment; the Android crash buffer was empty, which does not prove
a crash impossible. The listening window is complete and lead device tests may resume. Preserve the
owner's newer saves rather than restoring earlier test backups.

## Development takeover and coarse terrain — 2026-09-12

Owner transferred ongoing development and device integration to this lead. The other
session merged PRs #25/#26/#27 at `a91f7a0` (collision publication, shared audio adapter,
automatic-detail runtime). Their production wiring/device gates remain open where
recorded in the worker logs; no completion is inferred from module delivery.

The S2 coarse derivation slice is implemented: read-only bounded levels 1/2 tiles,
negative-coordinate mapping, deterministic materials, persistent deletion semantics
and explicit source revision/mode provenance. **27 core tests pass on the host and
physical OnePlus13 ARM64**, plus five independent public-API checks after correcting
a tester fixture. [Design, review and evidence](performance/coarse-terrain.md).
This is actual device CPU correctness evidence, not an APK visual/LOD acceptance.

PR #28 merged the coarse core at `c095047`. The scaled renderer example passes six
CPU tests and four software-Vulkan frames; independent Gemini review found no defects.
Terrain Lab passes 13 focused tests and the full app suite (125 passed, one ignored),
strict app Clippy and a four-frame host render. Lead chooser wiring is implemented;
Android build and independent integration review are underway.

Current DeepSeek workers integrate camera-driven wetland detail and shared audio
events in isolated checkouts. Lead owns their integration, Android deployment and acceptance. Core async coarse
publication, seamless LOD transitions and production landscape scale remain open.
Read the board before dispatch; no other session's implementation assignments remain
reserved after the owner transfer, but preserve its merged results.

## Autonomous microvoxel follow-up — implementation and reference assessment

[PR #23](https://github.com/5omeOtherGuy/Matterweave/pull/23) merged the
[autonomous roadmap](AUTONOMOUS_MICROVOXEL_ROADMAP.md) at `1e12264`. The disjoint
follow-up now adds two opt-in landscape fixtures to the real raster/ray/hybrid
comparison: thin near vegetation against a distant ridge, and negative-coordinate
orthographic terrain/opening edits. Default 16 runs and opt-in 24 runs pass host
software Vulkan validation. Independent image checks confirm visible edits and
near-detail oracle coverage; four GPU-free example tests, fmt and scoped strict
Clippy pass. [Review and provenance](performance/reviews/landscape-followup.md),
[fixture evidence](performance/landscape-functional-fixtures.md).

The first orthographic fixture exposed a 116-pixel top-row coverage mismatch.
Reframing passes without relaxed tolerances; the original remains a regression case,
not a fixed renderer defect. The [streaming research](performance/landscape-streaming-research.md)
recommended the narrow coarse-derivation experiment now delivered by PR #28.

The owner explicitly requires all runtime work on device. R01 and S5 now include a
network-disabled Android cold start, local generation/load/render/simulation/edit,
save, process restart and reload gate: **NOT RUN**. [Virtual Matter and Lay of the Land](performance/virtual-matter-and-lay-of-the-land.md)
were assessed for reusable technology and concepts; no offline Android SDK was
verified and neither engine is selected. Materials/fluids demonstrations do not
silently expand technical-freeze requirements.

No Android run or milestone closure was claimed for that fixture follow-up. At that
point the other session retained D1/D2/D4 and the phone lease (now transferred above). Global worker inspection showed its three
workers active during integration; these were preserved and no competing workers
were dispatched until capacity became available. Read the actual board and live
sessions before resuming, and merge that lead's updates instead of replacing them.
Next: lead-scheduled Android fixture execution; reserve coarse derivation ownership
before coding, without taking D2-owned core tests or app integration paths.

## Current reconciliation and completion plan — after PR #21

The planning baseline is `main = origin/main = 01ab450` (PR #21 merged), clean
checkout and no open PRs or live Pi workers at reconciliation on 2026-09-12.
E0 and E1 are closed: real AAudio 3/3 × 10 cycles at `a612f20`; detail-cadence
0 failures in 30 pinned suite runs, delivered by PR #21. Historical wave-1 branch
and routing descriptions below describe past attempts, not current assignments.

The owner requested a complete M0–M6 orchestration plan. Its updated
[execution contracts](ENGINE_COMPLETION_PLAN.md) retain M2–M6 as open and separate
phone-dependent acceptance from implementable fixtures, correctness and reuse work.
No engine or performance gate is closed by this documentation change. E2 remains
NOT RUN; the P02 thermal regression and human audibility remain open. The owner
subsequently confirmed sufficient battery and directed technical implementation
and functional Android acceptance first. E2 qualification, thermal campaigns and
performance optimization follow technical completion, with the original measured
M5 acceptance explicitly deferred. No new battery reading or device test is claimed.

Current routing: DeepSeek V4.1 Flash primary workers, independent Gemini initial
review waves, Opus only for justified escalation, no delegated Astra at any effort.
The [board](performance/board.json) owns current assignments.
Plan delivery: [PR #22](https://github.com/5omeOtherGuy/Matterweave/pull/22).
DeepSeek authored the plan; six initial Gemini code reviews were lead-triaged and
one independent final plan review passed. Documentation and diff checks pass;
[review dispositions](performance/reviews/completion-plan-v2-initial.md) record
verified gaps and rejected speculation. No new engine/device result is claimed.

## Completion execution — wave 1

Current integration branch: `codex/engine-completion-wave1-20260912`, [PR #19](https://github.com/5omeOtherGuy/Matterweave/pull/19).
Reconciliation preserved existing device evidence (`704bb4a`), completion-plan edits,
and the recovered audio worktree. Remote main was `065ff6e`; no supervised workers
or open PRs existed at dispatch. Historical assignments below are not live ownership.
The [board](performance/board.json) is the live authority; the lead owns the connected
OnePlus 13 and Android builds. GLM failed before inference on a provider parameter;
Hy4 timed out without a usable review. Neither is counted as completed participation.

Implemented:

- Audio separates service suspension from callback observation, preserves pause
  during recreation and failed recovery, reserves failed-pause compensation capacity,
  and retries output failures. PCM/voice reuse follows completed FIFO commands;
  unload targets the live generation. A separate fixed shared PCM allocation and
  raw mixer owner prevent control access through a live mutable mixer. Final source
  is `a999fd0`; both independent corrective reviews are resolved and the E1
  functional gate is accepted.
- E1 collision regression is accepted: caller-controlled publication, a wall above
  the autostep height, actual blocking before publication and movement afterward.
  Independent review findings are resolved; 11 native ARM64 tests pass in each of
  three final OnePlus 13 runs at `2d024f0`.
- Measurement tooling source is accepted after independent Muse/Gemini corrective
  reviews at `e436634`. Same-build ON/OFF collection preserves independent timing
  sources, validates generator/fixture/full-scene consistency, protects user files,
  rejects ambiguous or unstable pairs, and handles zero-noise/counterbalanced pairs.
  Current-build phone overhead and same-build noise remain **NOT RUN**.

Verification is recorded in the [final manifest](evidence/2026-09-12-completion-wave1/final-verification.json)
and [review triage](performance/completion-wave1-review-triage.md). At final source
`a999fd0`, 494 workspace tests pass (3 existing ignored gates), as do 49 native
ARM64 audio correctness checks and 4 × 20 real AAudio lifecycle cycles. Strict
workspace Clippy and formatting pass. Tool suites pass 251 performance tests and
16 coordination tests. Documentation checks pass. Muse/Gemini independently
reviewed the full ownership/lifetime correction and the final pause-intent fix;
all findings are resolved. PR #19 tracks the delivery revision and CI results.

The frozen debug APK (app runtime unchanged by this wave) built, passed native
16-KiB alignment/signature checks, and installed successfully. Source `f07c87c`,
SHA-256 `52a115b4a05e208c5d4c30629bb7e11d66202c856d8518a096457d2c13313506`.
Android reports keyguard covering the app; graphics qualification requires physical
unlock. Installation is not graphics execution. No human audibility, actual headset
disconnect, Miri, broader-device or sustained-efficiency claim is made.

Next: qualified ON/OFF phone collection after unlock and E3–E8 under the
[completion plan](ENGINE_COMPLETION_PLAN.md) and [gate ledger](performance/completion-gates-20260912.md).
Audio is not yet wired into the samples; M6 shared-service integration remains open.
The sections below retain earlier per-slice evidence and limitations.

## Production frame-loop scheduling — integrated and device-checked

Branch `codex/engine-frame-pacing` closes the roadmap's open "production frame-loop
scheduling" item and reassesses the unintegrated 30/60 Hz frame-cap experiment as
reusable engine work, per owner steering.

New crate `matterweave-pacing`: a deterministic, clock-free pacer (std only, no
`unsafe`, fixed 120-sample rings, no allocation after construction) that recommends a
present cadence and reports frame-timing statistics. It integrates the two evaluation
candidates preserved on `eval/pacing-muse` and `eval/pacing-glm` — Muse's structure and
public surface, GLM's typed configuration validation and deadline tolerance — and
corrects a defect **both** of them shared.

**The defect.** Both decided cadence from the presentation interval between frames. In a
loop that paces itself, that interval is the pacer's own recommendation plus the caller's
wait overshoot, so using it as the load signal closes a positive-feedback loop. Measured:
a simulated loop with a 200 microsecond overshoot ratchets to the slowest cadence within
three frames and stays there regardless of real cost, pacing a 5 ms workload on a 120 Hz
display down to 30 Hz; with zero overshoot the same loop holds correctly, which isolates
the feedback as the cause. `FrameSample` now carries the frame-boundary timestamp and the
frame production cost separately: intervals drive the statistics, cost alone drives the
cadence. `tests/closed_loop.rs` is the executable guard and fails if the two are
reconnected.

The wetland frame loop now uses it, replacing two hard-coded cadences (16.667 ms in world,
66.667 ms in menu) with a measured-cost-driven adaptive policy and an explicit idle policy.
The display period comes from the monitor and is rechecked about once a second, re-arming
only on a material change, because the OnePlus 13 panel advertises 120/90/60 Hz modes.

**Verification actually executed:**

| Check | Result |
| --- | --- |
| Workspace tests | PASS: `cargo test --workspace --locked` exit 0, 467 passed across 42 suites, 3 pre-existing ignored gates. |
| Strict Clippy / fmt / docs | PASS: `cargo clippy --workspace --all-targets --locked -- -D warnings` exit 0; `cargo fmt --all -- --check`; `python3 tools/check_docs.py`. |
| Mutation checks | PASS: every load-bearing assertion shown to fail when the behaviour it names is broken, then restored exactly. The lead independently reproduced the cost/interval-split, settle-window and single-sample mutations. |
| Host gate (real Vulkan) | PASS: `--pacing-check` under Xvfb/llvmpipe, 4 phases on a 16.667 ms period. |
| Device gate (OnePlus 13) | PASS: 4 of 4 phases, Adreno 830, Vulkan 1.3.284, APK `f7523ed8…`, source `c37f2e3`. `parked` recorded zero cadence changes across 240 frames; FIFO blocking measured at 2.3% of frame cost at worst. See [frame pacing](performance/frame-pacing.md). |
| Efficiency / thermal claim | NOT MADE: this is scheduling correctness only. No throughput, power, thermal or battery result, and no comparison against the previous fixed-cadence loop. |

**Review.** A four-perspective review swarm examined the change at a frozen revision. The
behaviour and interface reviewers returned no findings with substantive reasoning; the
lifecycle and test reviewers returned ten, of which the lead verified nine and rejected
one (a pre-existing, effectively unreachable busy-spin under frame-limited suspend, left
unfixed and recorded here). All verified findings are fixed on the branch.

## Detail-cadence test flakes — all three fixed

Three tests in `crates/matterweave-physics/tests/detail_cadence.rs` assert
`discarded >= 1` on the async detail worker. Two asserted it at a moment when the
discard had not necessarily happened yet and failed intermittently in CI, including
on a pull request that changed only Markdown. The production code was never at
fault; in both cases the behavioural guarantee under test held on every frame and
only the worker's bookkeeping raced. Fixed on 2026-09-11 (PR #17) by waiting for the
outcome under the existing `DEADLINE` instead of assuming the worker had already run.

**Reproduction.** Pinning the process to one CPU starves the worker deterministically
and turns the intermittent failure into a local one. Measured on the development
host: unmodified, `taskset -c 0` gave 15 failures in 20 runs against 0 in 20
unpinned; fixed, 0 failures in 150 pinned runs for each of the two tests.

**Mutation check.** With both supersede counters in `async_detail_collision.rs`
neutralised, three tests fail — the two repaired ones and
`stale_completion_for_an_edited_scene_is_never_published`, which was examined and
deliberately left unchanged because a publication has already been accepted by the
time it asserts. The source was then restored exactly and all 11 pass. This
establishes that none of the three assertions is vacuous; before the repair the
first could pass without exercising the discard path at all.

**The third is now fixed too** (PR #19).
`detail_cadence::edit_during_active_movement_blocks_until_publication_then_frees_the_path`
previously failed 3 times in 12 pinned runs with `the edit is pending: AsyncDetailStats
{ queued: 0, inflight: 0, results: 1 }`, because its assertion treated a buffered but
unpublished result as not pending and its guard required the worker to be slower than
the test.

The repair also exposed a second, larger defect the flake had been masking: the test's
wall fixture was a single cell 0.25 m tall, below the 0.30 m character autostep limit,
so the character could climb it. Because the old test only walked while the edit was
pending — usually tens of frames — it never pressed long enough to notice that its
"impassable" wall was passable. The test now uses a two-cell 0.5 m column and presses
for 600 frames, and it separates the withheld, completed-but-unpublished and published
phases so each is asserted without assuming worker timing.

Verified at `a612f20`: 0 failures in 20 pinned runs of that test, and 0 failures in 30
pinned runs of the full 11-test suite (`taskset -c 0`, single-threaded). The condition
that used to fail 15 times in 20 now passes every time.

## Capability status

| Capability | State | Evidence |
| --- | --- | --- |
| Production frame-loop scheduling | Integrated; host and OnePlus 13 device gates pass. No efficiency or thermal claim. | [Frame pacing](performance/frame-pacing.md) |
| Shadow-depth reuse | v0.4.0 prerelease; host Vulkan and OnePlus 13 functional checks pass. | [Shadow reuse](performance/shadow-reuse.md) |
| Automatic detail selection | v0.5.0; host and OnePlus 13 checks pass. | [Instance updates](performance/instance-updates.md) |
| Background indirect-light preparation | v0.5.0; host and OnePlus 13 checks pass. | [Background lighting](performance/async-indirect.md) |
| Bounded async collision, shared chunk snapshots, streaming stress, engine coverage, ray reference | Host and Android functional checks on the integration branch; not in a release. | [Collision log](performance/logs/engine-03-collision.md), [streaming](performance/stream-stress.md), [coverage](performance/engine-coverage.md), [ray reference](performance/ray-reference.md) |
| Bounded specular reflections | Merged (PR #13). Both OnePlus 13 app gates ran 2026-09-12: all phases completed; +7.45 ms GPU per frame when enabled. | [Reflections](performance/reflections-engine.md) |
| Bounded audio service | Merged (PR #12); suspend observability repaired (PR #19). OnePlus 13 diagnostic **PASSES** after the fix, 3 of 3 runs, 30 physical suspend/resume/shutdown cycles. | [ADR-0016](adr/0016-audio-service.md) |
| Voxel Relay second sample | Merged (PR #14). Physical device gate ran 2026-09-12: puzzle solved end to end on the OnePlus 13. | — |
| v0.3 slice: directional shadows, background preparation, destruction | Released (v0.3.0 development prerelease); device validated. | [v0.3 evidence](evidence/2026-09-08-v0.3.md) |
| v0.2 slice: walking, objects, streamed terrain | Released (v0.2.0); device exercised. | [v0.2 evidence](evidence/2026-09-07-v0.2.md) |

## Verified engine capabilities

### v0.5.0 — automatic detail and background indirect lighting

[v0.5.0](https://github.com/5omeOtherGuy/Matterweave/releases/tag/v0.5.0) is published
with APK, exact-source manifest and the retained lighting/detail/collision evidence
([release manifest](evidence/v0.5-release.json)). Delivered through PR9
(merged at `1cb468e` after host, Android and documentation checks passed at `4535b69`)
and PR10 (merged at `3ad0d27` after final-revision host, Android and docs CI passed at
`577dff8`).

- **Automatic detail selection**, integrated at `a4d7ba0` after 100 worker host tests. The
  renderer updates instance selection without recopying geometry; an 11-frame Vulkan check
  guards it ([evidence](performance/instance-updates.md)). The native adapter at `1abe19b`
  passes 9 Vulkan phases including perspective/orthographic zoom, edits and renderer
  recreation; OnePlus 13 Android 16 also passes 1080 frames, all 9 phases and HOME/resume.
  GLM numeric regressions and lead corrections pass 103 detail tests; an additional detail
  snapshot test passes separately. The PR9 code at `1abe19b` passes 326 workspace tests
  (3 existing ignored), strict Clippy, ARM64 APK inspection and a 1080-frame physical
  Android automatic-detail check including HOME/resume.
- **First bounded diffuse indirect reference**, integrated; its Vulkan shader passes host
  plus OnePlus 13 off/on/sun/enclosure/HOME-resume checks. Complete phase
  preparation/upload takes roughly 39–45 ms on that phone; scheduling and quality work
  remain. See [lighting evidence](performance/indirect-light-engine.md).
- **Bounded background indirect-light preparation** on branch
  `codex/engine-light-scheduling`. GLM's partial controller was recovered by the lead and
  Astra repaired its test assumptions. All 25 focused async tests pass at `2267ad5`; the
  native Vulkan adapter at `da96b8e` presented 15 frames while lighting was pending, then
  correctly published sun/edit/enclosure results. The combined 351-test suite and
  corrected strict Clippy pass. Production-controller coverage is 92.31% of lines with all
  27 functions exercised. The v0.5 ARM64 APK build/signature/alignment and physical
  OnePlus 13 Android 16 checks pass; the final capture records all six phases, nine
  presentations during preparation, HOME/resume and renderer recreation. The earlier
  secure-keyguard timeout remains a separate failed attempt. The APK source is `9854723`;
  the standalone ARM64 CPU check at `70a231a` matches every cell/face against synchronous
  lighting for an open, closed and reopened enclosure. A fresh local APK rebuild was
  byte-identical to the `9854723` artifact, passed signature/16 KiB checks, and passed
  another six-phase OnePlus 13 run with HOME/resume and nine presentations during
  preparation. See [background lighting](performance/async-indirect.md).
- Paid Muse Contributor reviewed the controller, queue lifetime, Vulkan publication and the
  standalone check without actionable findings; the lead owns the executed checks. Earlier
  quota notes are historical; a transient Muse 429 was retried successfully.

### v0.4.0 — shadow-depth reuse

The reusable Vulkan renderer reuses unchanged directional shadow depth and invalidates it
for geometry, light-matrix and resource changes, implemented at `78cba4a`. No sample
gameplay or save behavior changed in this engine slice. 257 workspace tests pass
(3 existing ignored gates), strict Clippy passes, and both 30-frame native Vulkan
cache/app checks pass without validation errors. The ARM64 APK `e4f83255…` built and ran
on OnePlus 13 Android 16: a 37-point continuous physics route passed, and 730 valid
profile rows include 284 completed shadow-map reuses and 445 depth updates. No measured
heat/energy improvement is claimed. See [implementation and evidence](performance/shadow-reuse.md).

The P02 collector finished all 3 matched pairs across A3/A4 and exited cleanly. The phone
is idle after the engine functional check; no collector or worker owns it. PR8 merged at
`ba6c894` after all required checks passed, and the
[v0.4.0 prerelease](https://github.com/5omeOtherGuy/Matterweave/releases/tag/v0.4.0) is
published with APK, manifests and both evidence archives.

### Integration branch — collision, snapshots, streaming and renderer reference

Branch `codex/engine-completion-03` (PR11 draft); the sole integration checkout is
`/mnt/bench/matterweave-dev/worktrees/performance-p00`. The
[board](performance/board.json) records ownership; the prior two-lead arrangement is
retired and this continuation owns phone, integration and delivery. Owner steering on
2026-09-08 reaffirms that the engine is the product: advance rendering, physics,
streaming, lighting and measured efficiency under [PERFORMANCE_PLAN](PERFORMANCE_PLAN.md)
and [PERFORMANCE_TASKS](PERFORMANCE_TASKS.md), while the [showcase](SHOWCASE.md) remains a
test workload whose further polish and old demo-save compatibility must not delay
remaining M2–M6 engine requirements.

- **Asynchronous collision.** At `70fc8bf`, fine collision can be prepared on a worker
  thread and published only against the same authoritative detail scene. Opaque scene
  versions distinguish unrelated/replaced scenes, forks and edits without hashing geometry
  or copying voxel payloads. Stale publication preserves current physics. Two version
  tests, 19 collision tests and strict detail/physics Clippy pass. At `6143e87`, the
  bounded asynchronous controller and corrected reset/reversal logic pass its
  queue/thread/contact tests. The direct ARM64 Android executable at `9cf26a1` passes
  asynchronous floor creation, standing contact, removal and falling on OnePlus 13.
  Frame-loop publication cadence is not yet integrated.
- **Shared chunk snapshots.** At `94e51eb`, world clones and streaming overrides share
  immutable chunk payloads. Changed chunks detach once; subsequent edits reuse their
  allocation until another snapshot shares it. Four allocation/behavior regressions and
  all core tests pass. See [chunk snapshot evidence](performance/chunk-snapshots.md). No
  mobile speed or memory measurement is inferred from allocation identity tests.
- **Streaming corrections.** Latest-request, reset and reversal corrections at `02fc5d3`
  pass 48 core tests.
- **Android packaging.** APK build/signature/alignment and indirect-light
  functional/lifecycle checks pass at `7211b7d`; later changes still require their own
  integration checks.
- **Streaming stress.** The ten-minute native streaming gate passed 39,761 cycles, 219,832
  meshes and 3,615 save/reloads within its declared limits
  ([summary](performance/stream-stress.md)). The same run sampled RSS between 7,052 and
  16,128 KiB and battery temperature rising 28.3 to 37.5 C with Android thermal status
  reaching 3; these are headless CPU-fixture observations, not a graphics-efficiency
  claim ([exact summary](evidence/2026-09-08-stream-stress.json)).
- **Engine coverage.** The frozen `b1f6c67` instrumented tests and four Vulkan examples
  passed. Per-object LCOV line union reports 94.62–97.01% across the four engine crates
  with explicit scope/diagnostics ([coverage evidence](performance/engine-coverage.md)).
  This is not Android/shader coverage or an engine-completion percentage. The stopped
  temporary coverage build target was removed; profile/report evidence was retained.
- **Ray reference.** The bounded authoritative ray-volume pack and Naga shader now execute
  through a headless Vulkan probe harness. All 73 renderer tests and strict scoped Clippy
  pass. The lead corrected 48 host synchronization hazards, then two Adreno precision
  failures; the expanded 30-probe suite now passes host synchronization validation and
  physical Android. The
  [manifest](evidence/2026-09-08-ray-reference.json) retains source/binary checksum and the
  complete phone report; see [ray reference](performance/ray-reference.md) for bounds and
  numerical tolerances. This is shader correctness evidence only; the full-image
  same-quality ray/raster/hybrid comparison and primary-path selection remain in progress.
- **Detail topology guard.** The guard now evaluates coarse fills sequentially. Independent
  fills had jointly closed a 2×2 tunnel; the reproduced regression and all-axis/negative
  variants now pass. Opus records 122 detail tests passing, including safe stepped wedge
  coarsening, preserved passages, aligned channels that safely reach Half, and per-instance
  edit invalidation. See [guard limitations and costs](performance/detail-local-loss-guard.md).
- **Collision cadence integration.** Production wetland edits queue collision preparation
  and poll publication before each simulation step. Added-solid regions defer publication
  while occupied; the workerless fallback applies the same gate. Rejection restores prior
  journal entries, and current rigid-body poses protect teleports before physics stepping.
  Eleven cadence tests and scoped strict Clippy pass. See
  [collision log](performance/logs/engine-03-collision.md). Native/Android integration
  validation of this revision remains pending.
- **Renderer comparison.** The engine-03 full-image comparison's initial 12-run Android
  gate passes at `bfb65ca`; the expanded 16-run host gate includes orthographic opening
  removal. It permits at most 0.05% CPU-proven face-edge ambiguity and zero unexplained
  mismatches. One/two edge pixels are recorded on the new fixture, not silently classified
  away. The one-cell descriptor regression is corrected; matched CPU oracle acceptance
  requires at least one non-excluded hit. Final Android repeat and combined workspace
  checks are running. [Protocol](performance/renderer-comparison.md).

### Merged slices and subsequent device validation

All three landed on `main` through pull requests #12, #13 and #14. Their code is in
the tree; what remains outstanding is device validation, not integration. The host
evidence below is unchanged.

**Bounded specular reflections** — `eval/hy4-reflections`, a separate worktree
`/mnt/bench/matterweave-dev/worktrees/eval-hy4-reflections` from frozen base `5b90975`,
adds an opt-in bounded specular reflection to the reusable Vulkan raster renderer. Merged
through PR #13; it changes no gameplay, audio, input or world system.

What exists: a `reflection` module (`ReflectionVolume`, `MaterialTable`, CPU oracle), two
new group-0 descriptor bindings, a `world.wgsl` single-bounce path,
`Renderer::upload_reflection/disable_reflection/reflection_enabled/reflection_state`, a
headless host validator, a real-Renderer smoke example and an app validation mode. Limits:
64³ source volume, configurable trace steps with a 512 hard maximum, one secondary ray, no
recursion or temporal history.

Host verification: 404 workspace tests pass (3 pre-existing ignored gates), strict Clippy
and `cargo fmt --check` pass, `tools/check_docs.py` passes, and the existing ray-reference
(30 checks), renderer-comparison (16 fixture runs), cache_smoke and indirect_smoke gates
all pass with no Vulkan validation errors. 28 predetermined non-edge probes and 346 seeded
randomized probes agree with an independent `World::raycast` oracle within 0.5/255
(3/255 tolerance); the nonreflective baseline is preserved within 0.5/255 (1/255
tolerance); recorded images show all seven required responses including an object outside
the camera frustum visible only through reflection; the real Renderer passes
enable/disable/edit/resize/recreation with an empty Vulkan validation stream. **Both device
app gates ran on 2026-09-12** and completed every phase: enabling reflections costs
+7.45 ms of GPU time per frame (4.501 -> 11.953 ms) and 50176 owned bytes, and the
invalidation contract holds on hardware — camera and sun motion republish nothing, while
each edit, removal and scene replacement republishes exactly once. Frame interval stayed
display-bound at ~16.6 ms throughout, so no frame-rate cost is visible at 60 Hz and none
is claimed at 120 Hz. See [reflection evidence](performance/reflections-engine.md)
and [draft PR 13](https://github.com/5omeOtherGuy/Matterweave/pull/13). This advances
R09/M4 and leaves ADR-0008 Proposed; it is not complete Lumen-like lighting.

**Bounded audio service** — `eval/glm-audio` (base `5b90975`) adds crate
`matterweave-audio` with a deterministic mixer core, a game-facing handle API
(register/play/stop/gain/suspend/resume, 32 clips, 8 voices, 4 MiB PCM, 64 commands,
explicit rejection instead of stealing/overwriting) and a real Android AAudio backend
through the pinned `ndk 0.9.0` bindings plus `ringbuf 0.5.1`. Decision record:
[ADR-0016](adr/0016-audio-service.md) (Proposed).

| Check | Result |
| --- | --- |
| Baseline at frozen base | PASS: `cargo test --workspace --locked` exit 0 before changes. |
| Crate behavioral tests | PASS: 22 tests (4 suites): two-voice fixture within 1e-6, limits at/over and after reuse, atomic rejection, stale handles, saturation/backpressure recovery, suspend/resume continuation policy, fault-injected device loss with recreation, threaded mixer interleaving. |
| Real-time allocation detector | PASS: zero allocations and deallocations across 10,000 mixer callback invocations including completion and stop handling (dedicated test binary). |
| Formatting / Clippy | PASS: `cargo fmt --check`; `cargo clippy -p matterweave-audio --all-targets -- -D warnings`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; Android-target scoped Clippy for the `backend-android` feature. |
| Workspace tests / docs | PASS: `cargo test --workspace --locked` exit 0 after changes; `python3 tools/check_docs.py` PASS. |
| Android cross-compilation | PASS: diagnostic example builds for `aarch64-linux-android` (debug and release) with the pinned NDK 28.2.13676358 API-28 linker, links `libaaudio`; release artifact SHA-256 `5b2a66036e6b94d1dacecaea013bef4344df603de29a0b23b75db3efce221c2b`. |
| Host diagnostic | PASS: negotiated properties, nonzero frames, suspend/resume continuation, controlled recreation, ten open/play/stop/close cycles (mock backend; silent by design). |
| Reserved-device diagnostic, before the fix | **FAIL** (OnePlus 13, 2026-09-12, 4 of 4 runs, build SHA-256 `578241740e71b724d9f9a6eeeeadcce9d23b9aaf1af861b5296bf2a68ec2210e`). Everything before the suspend phase passed on real AAudio: negotiated 48000 Hz stereo f32, burst 96, capacity 1536, low-latency, exclusive false; ~55 callbacks rendered 5088+ frames with 0 xruns and 0 device errors. The run then panicked at `audio_diagnostic.rs:133`, `render thread reports suspended`. Deterministic, not a flake. |
| Reserved-device diagnostic, after the fix | **PASS** (OnePlus 13, 2026-09-12, 3 of 3 runs at `a612f20`, build SHA-256 `b0b8cce87f2f3634d299f2c4d9e18f66e06caadaed5235dfa9ea2e2b30e346ed`). Each run completes 10 paused recreation/resume/shutdown cycles: 1101 callbacks, 105696 frames, 0 xruns, 0 device errors, 1 recreation. The split reports what hardware actually does — `suspended true, rt_suspended false` throughout every paused window, because AAudio delivers no callbacks while paused. Audibility remains a human observation; the counters prove submitted frames only. |

Limitations: no resampling and no compressed formats (48 kHz f32 mono/stereo only; a
device that cannot negotiate that fails open explicitly); a failed stream close aborts
through the ndk wrapper's drop contract; the on-device recreation check is a controlled
close/reopen, while true device-loss behavior is validated through shared-atomics fault
injection on host and the AAudio error-callback wiring compiled on device. The earlier
`engine-audio-service` worktree (Hy4 trial) was preserved untouched and not copied.

**Voxel Relay second sample** — `eval/gemini-voxel-relay`, a playable orthographic Android
puzzle sample proving the framework supports another distinct game genre without
duplicating the engine implementation.

- `matterweave_core::input::InputService`: a platform-independent multi-touch pointer and
  keyboard tracker in `matterweave-core`, reused by the explorer/wetland controls
  (`apps/explorer/src/controls.rs`) and Voxel Relay (`apps/explorer/src/voxel_relay.rs`)
  with zero engine code duplication.
- High-angle isometric chamber views via Vulkan `Renderer` (`Mat4::orthographic_rh` and
  `Mat4::look_at_rh`); on-screen virtual joystick and touch action buttons via `Hud`.
- Authentic `[2, 2, 2]` rigid body pushed across the stone floor using physical kinematic
  character impulses (`controller.solve_character_collision_impulses`); no teleportation or
  artificial forces.
- Occupancy of the pressure plate (`Z = 7.0`) removes door cells (`Z = 10.0`) in `World`,
  synchronizing compound static colliders in `Physics` via `sync_world`. The action button
  removes destructible obstacle voxels (`Z = 15.0`), opening the path to the exit zone
  (`Z >= 19.5`).
- Atomic persistence alongside world chunks via `World::save_with_attachment`, restoring
  player position, crate rigid body pose/velocity, door state, voxel edits and puzzle
  status.
- Verification: deterministic replay (`test_deterministic_replay_flow`) closes the door
  physically against the player (`9.5 < eye[2] < 9.75`), pushes the crate, opens the door,
  traverses the doorway, clears the obstacle and reaches the exit; save/reload
  (`test_save_reload_intermediate_and_unrelated_invariance`) restores intermediate state
  and leaves a pre-existing unrelated save 100% byte-identical; ten repeated replays
  (`test_ten_repeated_replays_deterministic_outcome`) produce identical world revisions and
  spatial endpoints within float tolerance; `test_input_service_comprehensive_edge_cases`
  covers simultaneous touch/action, cancellation, focus loss and recovery.
- Workspace tests pass; the explorer suite has 86 passed tests. `cargo fmt --all --
  --check` is clean and `cargo clippy --workspace --all-targets --locked -- -D warnings`
  reports 0 warnings. The debug APK built with Gradle 8.11.1 (`:app:assembleDebug`),
  SHA256 `aba3d489ad12d9e38573686e38fa8bd3f7646312029a1b78da9caf623e6e0165`, and
  `libmatterweave_explorer.so` ELF 16 KiB page alignment verified (`0x4000`).
- Physical device gate: **PASS** (OnePlus 13, 2026-09-12). Played end to end over
  wireless adb with injected touch events: the virtual joystick drove the kinematic
  character, the character pushed the crate onto the plate by impulse (crate settled at
  `[9.99, 1.50, 8.47]`), the door cells were removed and their colliders resynchronized so
  the character walked through the doorway (eye Z 9.68 -> 13.60 at X 6.52), the ACTION
  button cleared the destructible obstacle, and the character reached the exit at
  Z 20.83. Final saved state: `door_open true, obstacle_cleared true, solved true`, HUD
  `PUZZLE SOLVED`. Save attachment and screenshot in
  `/mnt/bench/matterweave-dev/device-gates/2026-09-12/`. An independent subagent verified
  engine/game separation, zero leakage into `crates/matterweave-*`, and the physics-backed
  tests.

## Known limitations, non-claims and open gates

No efficiency or thermal claim:

- Frame-loop scheduling is correctness only: no throughput, power, thermal or battery
  result, and no comparison against the previous fixed-cadence loop.
- Shadow reuse: no measured heat/energy improvement is claimed.
- Background indirect lighting: the functional checks do not establish full
  GI/reflections or sustained efficiency.
- Streaming: RSS and battery-temperature observations are from a headless CPU fixture,
  not a graphics-efficiency claim.
- Engine coverage is not Android/shader coverage or an engine-completion percentage.
- No sustained FPS, GPU-time, power or thermal superiority is claimed from the functional
  device tests. Frame/main counters are wall times; voxel payload and mesh buffer counters
  are not process RSS or free GPU memory.

Not implemented, not integrated or pending validation:

- Ray reference is shader-correctness evidence only; the full-image same-quality
  ray/raster/hybrid comparison and primary-path selection remain in progress.
- Collision publication has no fixed time bound while an addition is occupied; rendering
  may lead collision, and native/Android integration validation of the current revision
  remains pending.
- Detail: rough concavities remain conservative, and material-filled channels plus full
  temporal/quality M4 acceptance remain open.
- Reflections: 64³ source volume, configurable trace steps with a 512 hard maximum, one
  secondary ray, no recursion or temporal history; ADR-0008 remains Proposed; not complete
  Lumen-like lighting. Device cost is now measured per frame but not per joule: the
  2026-09-12 runs are seconds long and make no energy or thermal claim.
- Audio: no resampling and no compressed formats (48 kHz f32 mono/stereo only, with
  explicit fail-open on devices that cannot negotiate it); a failed stream close aborts
  through the ndk wrapper's drop contract. Suspension is now verified on hardware, but
  `rt_suspended` is structurally unobservable as true on any backend that stops delivering
  callbacks while paused — it stayed false across all 30 measured cycles, and no test may
  assert otherwise on a device. Audibility is still unverified: no human has confirmed
  sound from this device, and the counters only prove frames were submitted.
- Voxel Relay: physical device gate passes; the sample is playable and completable on the
  OnePlus 13.
- Full M2 equivalent-quality comparison, M3 stress gates, M4 indirect illumination,
  reflection and multiresolution-transition acceptance, a second released sample and
  broader device coverage remain open.
- Presentation teardown retains the previously documented Vulkan 1.1 WSI idle fallback.

v0.2 slice limits (still current unless superseded):

- Streaming and collision preparation are synchronous and can cause boundary-crossing
  stalls; background preparation is implemented, while collision publication, uploads,
  snapshot copying and saves remain synchronous. Cancellation is versioned and bounded.
- Saves cap overrides at 512 and total bytes at 12 MiB; new edits are refused at the cap
  while existing overridden chunks remain editable. Distant bodies freeze before collision
  eviction. Body/camera restoration validates finite bounded values.
- Frame/main counters are wall times; voxel payload and mesh buffer counters are not
  process RSS or free GPU memory.
- Real simultaneous multi-finger use, lock/unlock, process-memory pressure and additional
  devices are untested. ADB gestures here are sequential; unit tests cover concurrent touch
  roles, but that is not physical multi-finger validation. Phone Vulkan validation layers
  were unavailable; host validation does not cover its driver.

Owner decisions:

- Project licensing, production signing ownership and store publication remain owner
  decisions. v0.5.0 is a GitHub development prerelease, not a store build.

## Next concrete work

Follow the [engine completion orchestration plan](ENGINE_COMPLETION_PLAN.md),
written 2026-09-12 from the current repository evidence. It defines E0–E8,
dependencies, bounded parallel ownership and explicit M2–M6 acceptance gates.
Planning and documentation reconciliation do not close any engine gate.

1. Reconcile the current source, integration branches, PR state and historical board
   before dispatch; older branch assignments above are not a live ownership claim.
2. Both correctness items in this group are closed: the real-AAudio suspend failure is
   fixed (PR #19) and re-verified on the phone, 3 of 3 runs, and the detail-cadence
   observation race is fixed (PR #21) and re-verified under single-CPU pinning, 0
   failures in 30 suite runs. Qualifying mobile capture overhead and repeatability is
   the remaining work in this item.
3. Complete equivalent-quality ray/mesh/hybrid device comparison and primary-path
   selection; close production streaming/collision/destruction stress and bounds.
4. Complete GI/reflection quality, publication scheduling and stable detail
   transitions, measuring their combined costs. Reflections already pass the short
   device functional gates; that does not close M4 or sustained acceptance.
5. Finish reusable-service integration and acceptance for the existing wetland and
   Voxel Relay samples. Voxel Relay already passes end-to-end device play; audio,
   physical simultaneous touch, authoring/reuse and release acceptance remain.
6. Run matched, unplugged sustained tests on the final quality profiles and deliver
   the reviewed, merged Android demonstrator with durable evidence. Pacing and shadow
   reuse are implemented; measure their current behavior instead of assuming the
   historical unconditional-work baseline still applies.

Documentation-only planning validation: `python3 tools/check_docs.py` PASS (160
Markdown files, 539 local links, 16 ADRs, 20 requirements); `git diff --check` PASS.
No new engine,
APK or physical-device tests were run for the plan. M2–M6 remain open.

## Historical campaign records

The [completion execution log](performance/logs/completion-execution.md) and
[prior campaign status](STATUS_BEFORE_COMPLETION.md) retain earlier accepted
P00/P01/P02/P03 slices and unavailable measurement gates.

### v0.3 verified Android slice (2026-09-08)

The [v0.3 plan](V0.3.md) has working implementations of directional shadows, bounded
background terrain/mesh preparation and a six-body breakable arch that fully fractures to
64 pieces. The first APK was installed over v0.2 on the OnePlus 13; terrain, camera and old
bodies survived. New touch shadow/sun/detail/reset controls work, and the beam was
fractured on-device (6→29 bodies). Old saves receive default lighting preferences; changed
preferences persist.

The [execution log](../execution_log.md) records exact mixed-model assignments,
submissions, review corrections, board behavior and failed diagnostic hypotheses. Astra,
Opus 5 and Muse produced isolated contributions; the lead integrated them through
[PR #4](https://github.com/5omeOtherGuy/Matterweave/pull/4). The
[board](../tools/coordination/README.md) passed 16 local recovery/ownership checks and its
GitHub workflow. Submission is explicitly distinct from lead acceptance.

Current checks: 77 Rust tests pass (29 core, 17 explorer, 17 physics, 14 renderer),
workspace Clippy passes, and a 90-frame native app smoke passed edits, interaction, atomic
save/reload, resize and renderer recreation with Vulkan synchronization validation. ARM64
APK build/signature/16 KiB ZIP+ELF alignment pass. An exposed face in an isolated phone
fixture is pixel-identical on/off, including low sun at 2048; coarse terrace-shadow edges
remain a quality limit of the finite map.

A controlled 120-second warmup plus 20-minute fixed-quality run completed. All 61,509
selected presentation intervals were co-observed; mean 19.515 ms, p95 24.878 ms. All 40
health samples reported severe throttling. The run was USB powered and entered hot: no
causal speed/power comparison with v0.2 is valid. See the
[complete evidence](evidence/2026-09-08-v0.3.md) for exact conditions, clock/coverage
limits and CPU/GPU wall-time meanings. Final travel/reversal and two resume/relaunch cycles
passed; the user's saved scene was restored. GitHub CI passes; final delivery uses PR #4
and the v0.3.0 development prerelease.

The earlier v0.2 timestamp capture has 99.8183% verified interval-duration coverage; its
median/p95/p99 verified intervals were 16.580834/16.584323/16.585886 ms. Four gaps cross
missing dump histories and are not confirmed stalls. Thermal status reached SEVERE;
battery temperature 30.1→40.8 °C during its measurement window. These are recorded
observations, not a v0.3 comparison or power/thermal superiority claim.

### v0.2 interactive Android slice (2026-09-07)

M0/M1 shipped in v0.1; the owner reported that version working on a OnePlus 13. v0.2 was
directly exercised over USB on that phone with Android 16/API 36, LineageOS
23.2-20260818-NIGHTLY-dodge and Adreno 830 Vulkan 1.3.284. The [v0.2 scope](V0.2.md)
advances M2/M3; the full M2–M6 gates are not claimed complete.

- Material-preserving greedy chunk meshes, local revision invalidation, cached Vulkan
  buffers, conservative frustum culling and dynamic object draws.
- Connected terrain beyond the original island, bounded 7×7×3 residency, persistent edited
  chunk overrides and v0.1 migration that preserves removed chunks.
- Rapier walking, gravity/jump, exact edited-terrain collision, voxel rigid bodies, spring
  grabbing, throwing, bounded fracture and distant-body preservation.
- Touch walk/flight, object aim indicator, HOME, resettable objects, camera/control
  persistence and atomic combined world/object saves with corrupt-session recovery.
- Android repeat-launch protection, explicit Back handling and a narrow vendored winit
  lifecycle patch for destruction and sequential event-loop recreation.
- Version 0.2.0/code 2 ARM64 APK, optimization level 2, same local debug signing
  certificate as the v0.1 release. Dependency provenance and CI are updated.

Verification actually executed for v0.2 (see the
[v0.2 evidence report](evidence/2026-09-07-v0.2.md); historical v0.1 evidence remains in
the [original report](evidence/2026-09-07-mvp.md)):

| Check | Result |
| --- | --- |
| Workspace behavioral tests | PASS: 22 core, 14 explorer, 11 physics, 4 renderer; 51 total. |
| Formatting / Clippy | PASS: workspace/all-targets, locked, warnings denied for workspace code. One unchanged vendored upstream X11 function-cast warning remains a dependency warning. |
| Greedy/reference geometry | PASS: exact exposed unit-face/material equivalence and winding; identical fixture 27,744 → 6,660 triangles (76.0% reduction). Not a mobile FPS comparison. |
| Physics stress | PASS: 64 stacked bodies over 600 fixed steps; floor support, finite/restorable snapshot. Host test only. |
| Host app smoke | PASS: 30 presented frames, edits, grab/throw/fracture, atomic save/reload, actual resize and renderer recreation on llvmpipe with validation enabled. |
| Renderer cache smoke | PASS: six frames with submitted-buffer replacement/eviction, stale/invalid upload retention, culling, dynamic reuse/growth and teardown; no Vulkan validation errors. |
| Android build / signing / 16 KiB alignment | PASS: locked ARM64 APK; v2 debug signature; ZIP entry and all ELF LOAD segments verified, official zipalign passed. |
| OnePlus 13 install / controls / interaction | PASS: installed over v0.1; touch move/look/jump, grab, throw and eight-piece fracture; reset objects and aim indicator. |
| Device streaming / persistence | PASS: flew beyond original bounds to approximately (-8, 8, -63), placed a voxel, updated APK/reloaded; identical authoritative chunk data, camera and 18 bodies retained. |
| Device lifecycle / orientation | PASS: repeat launch, three Home/resume + Back/relaunch cycles in the same process, both landscape rotations and continued saving/rendering. Rotation overrides restored. |
| Documentation / dependency inventory | PASS; recorded source/licenses/checksums and exact winit patch. |

The public APK and its source/build manifest are delivered via
[GitHub Releases](https://github.com/5omeOtherGuy/Matterweave/releases). Remote PR
host/Android/docs checks must pass before merge under the owner's standing workflow. The
release tag identifies the final integrated source; its manifest records the exact build
revision and APK checksum.

### P00–P03 continuation checkpoints (2026-09-08)

- PR8 was draft/open at this point. All remote host, Android and docs checks pass at
  `3fe93f1`, including generator3 routes and native replay integration. Subsequent save
  restoration/analysis work is local; PR8 was not yet merged or released. (PR8 was
  subsequently merged and the shadow-cache APK device-checked and released, as recorded
  above.)
- Generator 3, seed 20260908, composition `dfb9f40519a3c151`: 34,716,467 expanded occupied
  cells, 8,899,364 unique stored cells, 6,192 flora/10 species and 5,721,300 expanded flora
  cells. Source meshes 47,255,040 bytes, within 64 MiB. These are host generated-data
  results, not mobile residency or performance claims. See the
  [manifest](evidence/full-wetland-generator3.json), including recorded routes.
- Graded paths and source-derived connectivity now pass actual continuous Rapier traversal:
  ground 632 points/197.46667 simulation seconds; elevated 37/11.35; waterside 21-point
  ground prefix/7.1166673. No jumps or intermediate teleports. The accepted 0.30 m autostep
  is used. An earlier 257-second trial silently missed six anchors and was rejected; every
  retained authored anchor now resolves or generation fails. All 19 prior source tests and
  the new waterside gate pass. Muse found no source blocker; Astra's waterside coverage
  finding was corrected.
- Workspace 257 normal Rust tests now pass with 3 deliberately ignored gates. The full-map
  Runtime gate was then explicitly run with the app suite after the restored-respawn
  correction: all 64 app tests pass. Strict workspace Clippy and the later scoped app
  Clippy pass; the vendored winit warning is unchanged. All 173 performance Python tests
  pass, including offline analysis and phone route-report checks. Native 0.4 Vulkan
  capture/replay cancellation also passes.
- Entrance correction validates the actual capsule with a bounded vertical lift. Generator 3
  prevents prior layout journals replaying against changed source. Corrupt/old-generator
  sessions select separate recovery files. Invalid edit references/body data now
  participate in actual recovery selection. Candidate source is isolated until collision,
  player pose, bodies and meshes all load. Full Runtime round-trip and rejection tests
  pass; see [restoration evidence](performance/showcase/save-validation.md).
- The generator3 native Vulkan/Xvfb/lavapipe capture passed 25 frames, 20 valid typed rows,
  six physics bodies, isolated persistence and no validation errors. Menu rendering is
  excluded from GPU completion joins. Source prototype geometry is shared on GPU; source
  LOD remains fixed. No automatic LOD or optimization win is claimed.
- The initial six-species full-map development APK (`fffb3844…`) was built, passed
  ARM64/16 KiB alignment and signature checks, and was installed on the OnePlus 13. Normal
  chooser/entry, movement, HOME/resume and persisted session were observed. The corrected
  ten-species APK (`35cbd2e6…`, source `7e1143e`) was installed; normal entry, movement,
  two source edits, process reload and HOME/resume rendering passed. Captures 1800/180 rows
  validate but show only renderer epoch 1.
  [Device evidence](evidence/2026-09-08-full-wetland-development.md) retains exact
  conditions and limits. Generator 3 development APK `4e47b6d9…` (`0d8225b`) installed and
  launched normally; touch move/jump and separate recovery 2 save observed. A separate
  owned clearing fixture passed touch fracture 6→29 bodies, settling and save. Full 64-piece
  stress and verified grab/throw remain open. The completed paired collector used frozen
  generator2 reference/candidate APKs; per-trial manifests identify the installed build.
  Full phone routes and shadow/temporal quality remain open.
- Actual app route replay is implemented and independently reviewed. Native Vulkan smoke
  advances it and records terminal cancellation correctly. The phone runner verifies
  complete route endpoints and deliberate touch/HOME cancellation; the elevated physical
  route now passes on the shadow-cache build. Ground and interruption checks remain
  pending.
- The 0.4.0 prerelease candidate at `8d8a3e8` built successfully in 61 s; APK
  `6f22ba87f7a9e3fbaac1c6763c17dd05f59fda1c4b192de329f3c9efd073faee` passes ARM64/16 KiB
  and signature checks. Artifacts are under `completion-02/release-candidate-04`. Not
  installed, merged or released. Gradle debug uses Cargo dev/opt-level 2/debug 0; the
  earlier release-profile label in development evidence was corrected against actual
  frozen source.
- P02 has three completed pairs across A3/A4; the first two are summarized here. Source,
  fixture, camera, shadows and entrance images match. Reference mean 73.551/75.820 ms;
  candidate supported mean 24.260/24.265 ms. A4 has nine history gaps (1.7547% of selected
  span); its whole selected-span mean is bounded above by 24.653 ms without inventing frame
  data. Candidate CPU and PSS are lower, but skin reaches 49.199/49.872 C and thermal
  status 2 versus reference 39.139/39.503 C/status 0. This is a throughput/thermal
  tradeoff, not a blanket efficiency win.
  [Analysis](performance/p02-analysis.md).
- A3 rejected trial 3 after 31 cooling observations, preserving its first pair. A4 completed
  the remaining two pairs with unchanged 1 C battery/2 C skin matching and exited with
  successful cleanup. Profiling overhead, motion/temporal quality and sustained
  final-build workload gates remain pending.
- The separate 30/60 Hz pacing experiment is preserved on its worker branch, unintegrated
  and unmeasured. Its worker has stopped; the pacing portion was later reassessed and
  closed as the integrated `matterweave-pacing` work recorded above. Further demo UI/save
  work is paused following owner steering. The lead discarded its own uncommitted
  grab/throw/break feedback UI patch. No measured thermal benefit from a cap is claimed.
- The [benchmark protocol](BENCHMARKS.md) and [showcase](SHOWCASE.md) remain the working
  scope; the numeric budgets in [PERFORMANCE_PLAN](PERFORMANCE_PLAN.md) are targets, not
  achieved results.
