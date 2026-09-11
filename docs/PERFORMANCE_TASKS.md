# Optimization tasks

Runnable backlog for the [performance campaign](PERFORMANCE_PLAN.md). The campaign
board recorded actual attempts, ownership, dependencies and accepted results. Every
implementation task includes its own engineering log and an observable definition of
done. Do not parallelize writers whose paths or interfaces overlap.

## P00 — Preflight and one coordination authority

- Inspect Git, profiles, toolchain, storage, existing processes, provider
  access/quotas and the device. Set paid ceilings before inference.
- Use exclusively owned fixtures and separate native saves; the owner revoked
  repeated save backup/restore rituals. Establish unplugged collection and cooling,
  brightness/refresh/quality setup. Keep phone ownership explicit.
- Start a new run; keep one authoritative board. Freeze accepted interfaces and
  record exact attempts and artifacts.
- Choose branch/worktree/build-cache layout and record the original baseline
  APK/source.

**Done:** one authoritative board; no overlapping ownership;
cancellation/replacement rehearsal retains partial work and rejects stale output;
route-selection checks recorded separately from model task qualification; phone
readiness or the exact unavailable gate recorded. No provider purchase or blanket
permission bypass.

## P01 — Trustworthy measurements and deterministic fixtures

- Explicit app/physics/render timing boundaries and work counters.
- Fixed-scene replay/fixture slice and behavioral tests in separate owned files.
- Manifest/measurement validator with positive and deliberately broken fixtures.
- Capture CPU busy time where supported, stage wall times and explicit waits; actual
  work counts; queue/collision/edit/save events; frame identities and missing samples.
- Version the small baseline, travel/reversal, settled/64-body scenes and capture
  modes.
- Preserve opt-in logging and normal uninstrumented operation.

**Done:** relevant format/behavior tests pass; malformed or mismatched captures are
rejected; missing data stays missing; retries and renderer recreation do not invent
frame joins; actual streaming jobs are distinguished from no-op request time; replay
ends at the expected versioned state within declared tolerances. Phone capture
overhead and same-build repeatability are measured before the system is used to
select optimizations. Host overhead is not substituted for mobile overhead. No
exact-energy claim without a qualified measurement source. Fixtures and
instrumentation require independent review before acceptance; a route probe does not
qualify an implementation.

## P02 — Remove redundant stationary and dynamic-object work

- Investigate the unconditional `dynamic_mesh` construction and `upload_dynamic`
  fence wait/rewrite, per-frame chunk bookkeeping, fixed physics work, redraw requests
  and periodic saves. Start with the measured dominant contributor.
- Design change tracking that accounts for interpolation, wake/sleep, grabs, fracture,
  restoration, residency, edits and renderer recreation. Do not rely on the mesh
  revision alone: the renderer treats it as informational for transforms.

**Done:** unchanged settled objects cause no unnecessary geometry reconstruction or
upload after warmup, while all listed changes invalidate correctly. Active motion and
interpolation remain correct; resting bodies wake when support or forces change. A
same-quality matched phone experiment meets the campaign improvement/no-regression
rule. Frame pacing and idle redraw are a separate candidate: zero background render
work, bounded wake-to-next-submission response (initial target <= one 60 Hz period,
excluding measured OS scheduling), no lost touch/input and no frozen active
simulation. Do not stop advancing a live world merely because the camera is
stationary.

## P03 — Representative dense tile and fine voxel representation

- Reusable voxel-volume/prototype transform and render/query interface.
- A representative 16 m tile and original initial flora with gallery fixtures.
- Independent occupied-cell/count/coordinate validators.
- Inspect viable existing representation, meshing and instancing techniques before
  custom work. Test a fine terrain/detail volume and repeated voxel plant prototypes,
  including negative coordinates, transforms, seams, edits, culling and collision
  policy.
- Keep world/object data authoritative and derived meshes versioned. Establish
  explicit CPU/graphics/staging/temporary budgets from this tile before full
  expansion.

**Done:** at least two meaningful source resolutions are queryable and rendered;
round trips and transform/boundary checks pass; fine source data survives LOD
changes; shared prototypes/instances and unique/expanded counts are correctly
distinguished. The phone gallery/tile looks detailed and yields measured
cost/peak-memory evidence. Accept an interface and storage/render path for the full
map without pretending this finishes the wider ray/mesh/hybrid comparison.

## P04 — Full original showcase

Work is split into terrain/hydrology, vegetation/scatter and mushroom-species asset
tasks, in successive waves; one bounded original mushroom prototype implementation,
then another localized experiment only if useful and within budget; scene load/render
integration and any measured instance/LOD bottleneck; scene manifest and
resource-budget validation; independent visual and code critiques after source/input
qualification. See [SHOWCASE](SHOWCASE.md).

Each asset worker receives the exact voxel scale, volume format, material IDs, bounds,
placement conventions, seed and gallery/geometry checks. Own separate generator or
asset files; one integrator owns the catalog and scene composition. A mushroom
definition of done includes recognizable silhouette and anatomy, validated
voxels/materials, deterministic variation and rendered views; counts alone cannot
pass it.

**Done:** the full map meets the showcase content, count, visual and route gates,
first at fixed quality before optional adaptation. Paid work records the actual
artifact, tests, lead repairs and credit evidence; failure is recorded and its task
rerouted, not hidden. Every model route has an actual task outcome or an explicit
access failure; no route needs to land code. Do not label early planning responses
as coding qualification.

## P05 — Active-work, streaming, memory and rendering loop

Select from measured costs; this is a prioritized menu, not permission to rewrite
all subsystems simultaneously.

- **Streaming/collision:** preparation copies, cancellation, fair queues, halo/mesh
  invalidation, collision publication, staging and incremental upload budgets.
  Done: fast reversal/edits/eviction reject stale jobs; resident support remains
  valid; declared queue/allocated-capacity bounds hold under stress; measured travel
  tail improves.
- **Rendering:** per-object/static geometry reuse, instance submission, culling/LOD,
  shadow invalidation/caster bounds, pass cost, overdraw and synchronization
  placement. Done: lifetime validation, offscreen casters, camera/light changes,
  water/transparency, resize and old-result retirement pass; matched images and
  motion preserve accepted quality. Extra frames in flight or cache layers require
  evidence, not presumed speedups.
- **Physics:** active/sleeping body work, broad/narrow phase and character/query
  cost. Done: contacts, support removal, grab/throw, 64-piece fracture, serialization
  and fixed-step/interpolation behavior remain valid; do not disable simulation to
  win.
- **Storage/startup/memory:** scene decoding, allocation/reuse, save
  copies/compression, prototype residency, resource retirement and repeated
  lifecycle/reset/load. Done: exact authoritative round trips and old-save recovery
  pass; cold start and peak memory are measured; no monotonic leak beyond declared
  bounded caches across 20 repeat cycles after warmup, with OS-accounting uncertainty
  explicitly reported.

For each chosen row, write a distinct hypothesis, metric and definition of done before
dispatch. When the bottleneck changes, return to measurement rather than finishing a
stale optimization checklist. Keep showcase correctness and density fixed during
comparisons.

## P06 — Water/material/vegetation finish and explicit quality profiles

- Bounded material and asset work; water surface/render integration and measured
  quality-control work; appearance, thin-feature, transition-artifact and cost-claim
  reviews.
- Complete coherent water and the original lush visual identity.
- Separate static work reduction from optional wind, transparency/reflection, LOD or
  resolution adaptation.
- Evaluate a 60 Hz responsive and an explicitly labeled 30 Hz fidelity profile if
  useful; show what changed. Do not silently reduce the source voxel/vegetation
  requirement.

**Done:** banks, surfaces and player interaction are coherent; flora and mushrooms
pass recorded near/far views; temporal artifacts and worst-case vistas are reviewed;
water/vegetation cost is measured on the phone. Player UI exposes useful controls
with diagnostic counters opt-in. Full GI, fluid simulation and broad renderer
research are not implied requirements for this campaign; report their status
honestly.

## P07 — Combined acceptance and release

- Independent final review waves first, then final code/evidence review, integration
  and device ownership. Record any specific remaining lifetime/invalidation audit
  separately.

**Done:** all required host/Android checks pass; final source and quality fixtures are
frozen; short matched comparisons and finalist 20-minute sustained phone tests cover
the actual showcase stationary, movement and destruction workloads. Report
variance/coverage, thermal conditions, performance/quality tradeoffs, peak memory and
unmeasured quantities. No critical review finding remains unresolved. The APK matches
its published checksum, installs over the development release and preserves user
saves. PRs are merged and the GitHub prerelease includes APK, build/scene manifests,
screenshots/video, reports, checksums and final engineering logs. STATUS names the
exact remaining limits and the next runnable action; the user's scene is restored and
phone tests stop.
