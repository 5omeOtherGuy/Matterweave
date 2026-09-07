# Optimization execution tasks and model assignments

This is the initial runnable backlog for [the campaign](PERFORMANCE_PLAN.md).
It is not a second live board. During execution the lead records actual attempts,
ownership, dependencies and accepted results in the one chosen campaign board.
Assignments below are proposed; reroute after observed failures/capacity changes
while preserving the all-model qualification obligation. Do not parallelize writers
whose paths or interfaces overlap. Every implementation task includes its own
engineering log and observable definition of done.

## P00 — Preflight and one coordination authority

**Lead owner:** Astra. **Helpers:** one bounded Muse or GLM worker only for a concrete
adapter/test task; no agent should write another orchestration plan.

- Inspect Git, current skill/profile settings, toolchain, storage, existing processes,
  provider access/quotas and the device. Set Hy4 paid ceilings before inference.
- Preserve user saves; establish an unplugged collection route and repeatable cooling,
  brightness/refresh/quality setup. Keep phone ownership explicit.
- Start a new run. Reconcile the current SQLite board's committed-submission contract
  with Pi's patch handoff using a minimal tested adapter, or use a sole lead-written
  bounded board. Freeze accepted interfaces and record exact attempts/artifacts.
- Choose branch/worktree/build-cache layout and record original baseline APK/source.

**Done:** one authoritative board; no overlapping ownership; cancellation/replacement
rehearsal retains partial work and rejects stale output; actual route selection checks
recorded separately from model task qualification; phone readiness or exact unavailable
gate recorded. No provider purchase or blanket permission bypass.

## P01 — Trustworthy measurements and deterministic fixtures

**Opus:** explicit app/physics/render timing boundaries and work counters.
**Muse:** fixed-scene replay/fixture slice and behavioral tests in separate owned files.
**GLM:** manifest/measurement validator with positive and deliberately broken fixtures.
These tasks run concurrently only after the measurement/fixture schema is frozen.

Capture CPU busy time where supported, stage wall times and explicit waits; actual
work counts; queue/collision/edit/save events; frame identities and missing samples.
Version the small baseline, travel/reversal, settled/64-body scenes and capture modes.
Preserve opt-in logging and normal uninstrumented operation.

**Done:** relevant format/behavior tests pass; malformed/mismatched captures are rejected;
missing data stays missing; retries/renderer recreation do not invent frame joins;
actual streaming jobs are distinguished from no-op request time. Replay ends at
expected versioned state within declared tolerances. Phone capture overhead and
same-build repeatability are measured before using the system to select optimizations.
Host overhead is not substituted for mobile overhead. No exact-energy claim without
a qualified measurement source.

**Qualification:** Muse/GLM handoffs include independently checked fixtures; Opus's
instrumentation is reviewed by Muse/Gemini before Astra acceptance. A route probe
alone does not mark any implementation model qualified.

## P02 — Remove redundant stationary and dynamic-object work

**Primary writer:** Opus. **Independent Muse lane:** invalidation/regression fixtures.
**GLM lane if useful:** counter/report tests after the schema is stable.

Investigate the current unconditional `dynamic_mesh` construction and
`upload_dynamic` fence wait/rewrite, per-frame chunk bookkeeping, fixed physics work,
redraw requests and periodic saves. Start with the measured dominant contributor.
Design change tracking that accounts for interpolation, wake/sleep, grabs, fracture,
restoration, residency, edits and renderer recreation. Do not use the current mesh
revision alone: the renderer explicitly treats it as informational for transforms.

**Done:** unchanged settled objects cause no unnecessary geometry reconstruction or
upload after warmup, while all listed changes invalidate correctly. Active motion
and interpolation remain correct; resting bodies wake when support or forces change.
A same-quality matched phone experiment meets the campaign improvement/no-regression
rule. Frame pacing/idle redraw policies are a separate candidate: zero background
render work, bounded wake-to-next-submission response (initial target<=one60Hz period,
excluding measured OS scheduling), no lost touch/input and no frozen active simulation.
Do not stop advancing a live world merely because the camera is stationary.

## P03 — Representative dense tile and fine voxel representation

**Opus:** reusable voxel-volume/prototype transform and render/query interface.
**Muse:** representative16m tile and original initial flora with gallery fixtures.
**GLM:** independent occupied-cell/count/coordinate validators.

Inspect viable existing representation/meshing/instancing techniques before custom
work. Test a fine terrain/detail volume and repeated voxel plant prototypes, including
negative coordinates, transforms, seams, edits, culling and collision policy. Keep
world/object data authoritative and derived meshes versioned. Establish explicit
CPU/graphics/staging/temporary budgets from this tile before full expansion.

**Done:** at least two meaningful source resolutions are queryable and rendered;
round trips and transform/boundary checks pass; fine source data survives LOD changes;
shared prototypes/instances and unique/expanded counts are correctly distinguished.
Phone gallery/tile looks detailed and yields measured cost/peak-memory evidence.
Accept an interface and storage/render path for the full map without pretending this
finishes the wider ray/mesh/hybrid comparison.

## P04 — Full original showcase and every-model implementation trial

**Muse workers:** separate terrain/hydrology, vegetation/scatter and mushroom-species
asset tasks, in successive waves. **Hy4:** one bounded original mushroom prototype
implementation, then another localized experiment only if useful and within budget.
**Opus:** scene load/render integration and any measured instance/LOD bottleneck.
**GLM:** scene manifest/resource-budget validation. **Gemini/Muse:** independent visual
and code critiques after source/input qualification. See [SHOWCASE](SHOWCASE.md).

Each asset worker receives exact voxel scale, volume format, material IDs, bounds,
placement conventions, seed and gallery/geometry checks. Own separate generator or
asset files; one integrator owns the catalog and scene composition. A mushroom DoD
includes recognizable silhouette/anatomy, validated voxels/materials, deterministic
variation and rendered views; counts alone cannot pass it.

**Done:** full map meets the showcase content/count/visual/route gates, first at fixed
quality before optional adaptation. Hy4's actual artifact/tests/lead repairs and
credit evidence are recorded; failure is recorded and its task rerouted, not hidden.
All six models have actual task outcomes or explicit access failures. No need for
each to land code. Do not label early planning responses as coding qualification.

## P05 — Active-work, streaming, memory and rendering loop

Select from measured costs; this is a prioritized menu, not permission to rewrite all
subsystems simultaneously. Opus owns consequential core/render changes. Muse handles
independent localized optimizations/tests. GLM and Hy4 get bounded alternates where
qualification and budget support them.

- **Streaming/collision:** preparation copies, cancellation, fair queues, halo/mesh
  invalidation, collision publication, staging and incremental upload budgets.
  Done: fast reversal/edits/eviction reject stale jobs; resident support remains valid;
  declared queue/allocated-capacity bounds hold under stress; measured travel tail improves.
- **Rendering:** per-object/static geometry reuse, instance submission, culling/LOD,
  shadow invalidation/caster bounds, pass cost, overdraw and synchronization placement.
  Done: lifetime validation, offscreen casters, camera/light changes, water/transparency,
  resize and old-result retirement pass; matched images/motion preserve accepted quality.
  Extra frames in flight or cache layers require evidence, not presumed speedups.
- **Physics:** active/sleeping body work, broad/narrow phase and character/query cost.
  Done: contacts, support removal, grab/throw,64-piece fracture, serialization and
  fixed-step/interpolation behavior remain valid; do not disable simulation to win.
- **Storage/startup/memory:** scene decoding, allocation/reuse, save copies/compression,
  prototype residency, resource retirement and repeated lifecycle/reset/load.
  Done: exact authoritative round trips/old-save recovery pass; cold start and peak
  memory are measured; no monotonic leak beyond declared bounded caches across20
  repeat cycles after warmup, with OS-accounting uncertainty explicitly reported.

For each chosen row, write a distinct hypothesis, metric and DoD before dispatch.
When the bottleneck changes, return to measurement rather than finishing a stale
optimization checklist. Keep showcase correctness and density fixed during comparisons.

## P06 — Water/material/vegetation finish and explicit quality profiles

**Muse/Hy4:** bounded material/asset work according to qualification.
**Opus:** water surface/render integration and measured quality-control work.
**Muse/Gemini reviews:** appearance, thin features, transition artifacts and cost claims.

Complete coherent water and the original lush visual identity. Separate static work
reduction from optional wind, transparency/reflection, LOD or resolution adaptation.
Evaluate a60Hz responsive and an explicitly labeled30Hz fidelity profile if useful;
show what changed. Do not silently reduce the source voxel/vegetation requirement.

**Done:** banks/surfaces/player interaction are coherent; flora/mushrooms pass recorded
near/far views; temporal artifacts and worst-case vistas are reviewed; water/vegetation
cost is measured on the phone. Player UI exposes useful controls with diagnostic
counters opt-in. Full GI, fluid simulation and broad renderer research are not implied
requirements for this campaign; report their status honestly.

## P07 — Combined acceptance and release

**Swarm:** Muse/Gemini focused final review waves first. **Astra:** final code/evidence
review, integration and device ownership. Use a Pi Astra medium worker for a specific
remaining lifetime/invalidation audit when useful; record its outcome separately.

**Done:** all required host/Android checks pass; final source/quality fixtures are frozen;
short matched comparisons and finalist20-minute sustained phone tests cover actual
showcase stationary/movement/destruction workloads. Report variance/coverage, thermal
conditions, performance/quality tradeoffs, peak memory and unmeasured quantities.
No critical review finding remains unresolved. The APK matches its published checksum,
installs over the development release and preserves user saves. PRs are merged and
GitHub prerelease includes APK, build/scene manifests, screenshots/video, reports,
checksums and final engineering logs. STATUS names exact remaining limits and next
runnable action; the user's scene is restored and phone tests stop.
