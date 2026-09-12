# Mesh lighting review disposition — 2026-09-12

## Renderer API review

Gemini w_0a5e161c reviewed frozen80a89c5. Lead verified all three candidates:

1. **Accepted and fixed:** upload_dynamic's in-place rewrite disabled GPU reflection
   but retained Renderer publication/presentation tokens. Calling disable_reflection
   before writes now retires both tokens even on reuse/failure. An existing native
   reflection_smoke scenario exercises the empty dynamic fast path: before the fix
   it failed with published Some(1), expected None; after the fix the full native
   smoke passed. This is metadata correctness; stale reflection rendering was already
   disabled before this fix. Nonempty rewrite uses the same invalidation entry.
2. **Rejected as a correctness defect:** reflection checks source/digest before
   mesh coverage, so a multiply-invalid request can receive a stale-source error
   instead of the coverage error. Both paths disable and reject publication; no
   contract promises error-string precedence. No behavior change required.
3. **Recorded for Phase B:** upload_indirect's wait is not included in the upload
   fence tally. The direct wait predates this API change. Timing attribution remains
   deferred with performance work; do not treat the tally as complete GI upload cost.

## Native diagnostic review

Gemini w_1b4b44aa reviewed c043ed5. Lead accepted both findings: the fixture's world
floor/receiver existed for CPU queries but was never uploaded to the GPU, and the
cells label printed a material ID. Muse w_1c518c32 is correcting these in its original
isolated checkout. Initial host PASS established publication/control logic, not
world rasterization or final visual correctness. Corrected evidence and Android
acceptance remain pending.

## Checks and remaining gates

Lead regression commands used RUSTC_WRAPPER empty, one Cargo job and healthy
mesh-native-target: xvfb-run -a cargo run --locked -p matterweave-render --example
reflection_smoke. Red failed at the publication-token assertion; green completed
all stages. Raw logs are in the healthy recovery orchestration directory as
reflection-rewrite-red.log and reflection-rewrite-green.log. No device/performance
claim is made by this host check. Corrective diagnostic review, combined native
Android acceptance and passing PR36 CI remain required.

## Native corrective result

Muse b7b39bf uploads the authoritative world on every renderer creation and
recolors it with the fixture palette. Lead inspected the upload/recreation path,
boundary-cell lookup (fixture-local, not a general authoring API) and corrected
occupied-cell label. The worker's 11 focused checks and native30-frame run passed;
GPU accounting retains42336 world bytes after removing static instances. This
resolves both initial diagnostic findings at host level; combined Android visual
acceptance remains pending. Original field defaults and proxy identity are unchanged.

## Owner-requested GLM 5.3 Flash review

Reviewer `w_46c2ce3f`, exact OpenRouter `z-ai/glm-5.3-flash`, high, completed
review of45ab5de versus main495d9f9, resuming preserved source-reading context
from the owner's initially specified full GLM5.3 route. Full-GLM attempts alone
are not counted as completed reviews. No Astra worker was used.

- F1 (proxy attached after rendered geometry removed): the methods validate
  caller-supplied identity, not GPU-derived coverage. This was an explicit caller
  responsibility, and a blanket rejection would also reject valid empty proxies.
  Kept the API behavior; clarified both method contracts that obsolete cells must
  be removed and an old digest cannot establish current-scene correspondence.
  This remains a documented caller precondition, not automatic coverage proof.
- F2 (report-write panic): confirmed. Replaced the report sink panic with a
  platform error log and terminal failed state. Event-loop handling exits cleanly;
  later PASS records are suppressed. Deterministic tests cover unavailable sink
  at startup and failure after a successful initial report; no permission-based
  test that would spuriously pass as root.

The baseline candidate passed physical Android five-phase publication and
HOME/resume before this report-error handling correction. Relevant corrective
host checks and final APK checks are recorded in STATUS; no full-production GI,
subcell identity or thermal acceptance is inferred.
