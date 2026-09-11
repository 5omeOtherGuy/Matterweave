# Lead native/P02 integration judgment

## Actions

Integrated accepted flora source 64661cf/evidence 267c2c8 through PR7, merged c71306d
after host/Android/docs CI. Checked frozen revision/path scope, not a duplicate
source audit. Added SourceLOD native flora and CPU-cache/GPU-residency integration.
180 Rust tests passed; real host gallery captures and 90-frame normal Vulkan
lifecycle smoke passed. Code checkpoint 6fac9a5 is accepted for phone evaluation,
not accepted as a measured optimization or full native showcase.

## Independent findings and corrections

- Initial Muse found no cache/lifecycle regression. Its len-versus-capacity note
  is retained: 64 MiB checks logical combined payload, not a strict allocation cap.
  Do not dismiss this by referring only to allocator metadata exclusions. Current
  two fixed built-in scenes are the supported scope; generic large-scene
  allocation safety/instancing needs a subsequent gate.
- Gemini identified a real metric error: the gallery counted its static
  compatibility upload as dynamic-body work. Actual native 30-frame test produced
 20 schema-valid rows and failed on dynamic uploads 1.0155f3b/6fac9a5 preserve
  the runtime RED/GREEN. Corrected dynamic counters 0, dynamic timings absent:
  there is no dynamic pipeline, unlike supported-but-skipped normal-game work.
- Multiline extra directives also reproduced as accepted and now fail closed.
- Both independent final reviews found no new concrete correction/WSI defect.
  Reject overbroad prose: acquire SUBOPTIMAL does not guarantee a later successful
  present; code treats them separately. One observed Adreno/device result is not
  proof for every driver. Existing documented exceptional WSI retirement limits
  are not magically resolved by queue-idle wording in a review.
- The acceptance checker subsequently gained a two-line optional `--preset flora`
  selector and passed against the real native 85-instance scene. Production Rust
  remained at the reviewed 6fac9a5; this test-only extension is lead-verified, not
  misrepresented as part of that exact frozen commit.

## Evidence-led performance work

Three recorder-overhead pairs completed, with explicit whole-process-rate and
independent-clock limitations. A separate 16-draw diagnostic established repeated
successful-but-suboptimal presents and roughly 16ms swapchain reconstruction.
Khronos permits continuing with SUBOPTIMAL; 57b6122/21c060c record runtime RED/GREEN
for deferring advisory reconstruction while retaining resize/OUT_OF_DATE paths.
Shader/quality/rotation policy is unchanged. Phone comparison remains pending.

## Issues and solutions

The first diagnostic restoration verification failed; the original backup was
retained, explicit post-install stop and synchronous shell writes restored it
with two byte-identical reads. Root cause was not proven. A corrected second
trial streamed app-UID logs before launch and restored APK/save successfully.
No Android security setting was weakened after simpleperf's denied attempt.

Sharing the main Android target with the older probe worktree reused stale
physics metadata: source exported DynamicMeshCache, but compiler couldn't import
it and dependency output listed only lib.rs, not dynamic_cache.rs. No Cargo build
was active before cleaning only Android workspace-package artifacts; third-party
and host caches were preserved. Rebuild passed with fresh local crates. Do not
reuse compiled local targets across differing worktrees again.

## Next gates

Phone native normal/flora lifecycle, actual appearance, cache work counts and
matched P02 performance; source collision integration; remaining P01 counters/
replay and P04–P07/full-map showcase. Raw overhead artifacts remain local with
small durable summaries/hashes, not a claimed remote artifact publication.
