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
