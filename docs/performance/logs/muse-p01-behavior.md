# P01 instrumentation review — read-only leaf (NOT RUN)

Scope: frozen candidate at `/mnt/bench/matterweave-dev/performance/run-01/p01-candidate-01` (patch + `sha256.json` taken as given; hashes not re-verified — no execution). Changed files read from frozen candidate; `timing.rs` read as unchanged supporting source from `/mnt/bench/matterweave-dev/worktrees/performance-p00`. No execution checks performed; all verification is by source reading. **NOT RUN**: no tests, builds, device runs, or repeatability/overhead measurements.

Verdict: instrumentation-only contract largely holds (gameplay/save/lifecycle paths preserved, opt-in semantics intact, integer identities with epoch, empty-not-zero, no compositor join, GPU span honestly labeled as queue timestamps). 4 concrete doc-vs-code findings below; no optimization implementation proposed.

## Finding 1 — Failed saves counted as completed (`saves_performed` / `save_wall_ms`)
- **File:line**: `apps/explorer/src/lib.rs:303-348` (match at ~330-345, unconditional increments at :347-348); doc `docs/performance/measurement-v2.md:146`.
- **Trigger**: any save failure during an active capture (I/O error, missing parent — the existing `failed_autosave` path proves failures are reachable).
- **Consequence**: `saves_performed` overcounts completions and failed-attempt wall time is attributed as save work; P02 could misread save frequency/cost.
- **Source evidence**: `save()` matches `Ok/Err` (Err sets `SAVE FAILED` status) then unconditionally executes `self.save_ms += …` (:347) and `saves_since_capture.saturating_add(1)` (:348). Doc defines `saves_performed` as "Saves completed since the previous row".
- **Uncertainty**: low on mechanism; intent uncertain — counting attempts may be deliberate, but then the doc word "completed" is wrong.
- **Discriminating check (NOT RUN)**: capture-active unit test with a failing save path (missing parent) asserting `saves_performed`/row; fix is either gate the counter on `Ok` or reword doc to "attempted".

## Finding 2 — `present OUT_OF_DATE` counted as `Presented`, doc says "successful present API calls"
- **File:line**: `crates/matterweave-render/src/lib.rs:1544-1558` (present match; `Err(OUT_OF_DATE) => recreate=true` falls through to `Ok(Presented)`); contrast acquire-side `:1401` which correctly returns `Retry`; doc `docs/performance/measurement-v2.md:68`.
- **Trigger**: surface change (resize/rotation/suspend-resume) making `vkQueuePresentKHR` return `ERROR_OUT_OF_DATE_KHR`.
- **Consequence**: `presented_count` advances and the row is labeled `presented` though the image was likely not presented; the submission is joined as presented work and a real retry-age signal is lost.
- **Source evidence**: patch context shows this fall-through is pre-existing (candidate only wrapped timing around it), while the new doc formalizes "successful `vkQueuePresentKHR` call" — the doc is stricter than the code.
- **Uncertainty**: medium — some drivers may still display on OUT_OF_DATE; base-commit comparison not done (read-only, no git execution); changing the mapping would alter lifecycle accounting, so a doc correction may be preferable to a code change.
- **Discriminating check (NOT RUN)**: source-compare base `75562047` present-match arms; needs a device to observe (headless test impossible); lead decision: doc fix ("returned without device error") vs mapping OUT_OF_DATE to `retry`.

## Finding 3 — `main_cpu_busy_ms` stopwatch outlives `main_wall_ms`; doc claims "same span"
- **File:line**: `apps/explorer/src/lib.rs:1069` (wall stopped) vs `:1070-1088` (gpu/diagnostics/shadow reads, then busy end at :1085-1088); doc `docs/performance/measurement-v2.md:88`.
- **Trigger**: every captured row.
- **Consequence**: busy interval includes `gpu_timings()`, `draw_diagnostics()`, `shadow_caster_meshes()` plus row preamble after the wall stopwatch; busy can exceed wall by microseconds, which could false-flag a strict busy≤wall validator and slightly bias P02 wall-vs-busy deltas.
- **Source evidence**: `self.cpu_ms = now.elapsed()` (:1069) is assigned before the completion/diagnostic/shadow reads and `cpu_busy_ms(begin, thread_cpu_time())` (:1085); doc says busy is "over the same span".
- **Uncertainty**: magnitude unmeasured (expected µs; NOT RUN); direction is certain from statement order.
- **Discriminating check (NOT RUN)**: take the CPU end reading before computing the wall end, or document the distinct stop points and ordering guarantee.

## Finding 4 — Counters doc says "missing only when the source was unavailable"; per-completion columns are routinely empty
- **File:line**: `docs/performance/measurement-v2.md:134` vs row construction `apps/explorer/src/lib.rs:1095-1115` (`completed_gpu_frame_id: gpu.map…` :~1096-1098, `gpu_prev_*: gpu.map/and_then` :~1110-1115); doc's own identities section correctly says completed is empty with no new completion.
- **Trigger**: any presented frame whose submission isn't complete yet (`NOT_READY` clears to `None`) and all retry rows.
- **Consequence**: a consumer trusting :134 reads empty `completed_*` / `gpu_prev_shadows` / `gpu_prev_shadow_map_size` as "renderer unavailable" instead of the normal "no new completion this attempt" pacing gap. Code behavior (empty, not zero) is correct; the doc blanket statement is internally inconsistent.
- **Source evidence**: `gpu` is `None` unless `GpuCompletionTracker::accept` sees a new completion, and all `gpu_prev_*`/completed cells derive from it.
- **Uncertainty**: low — doc-internal inconsistency only.
- **Discriminating check (NOT RUN)**: doc edit qualifying `completed_*`/`gpu_prev_*` as empty when no new completion observed; consumer test parsing a no-completion row.

## Engineering log
- **Actions Taken**: read frozen `measurement-v2.md`, `opus-p01-a1.md`, `metrics.rs`, explorer `lib.rs` (draw/save/lifecycle), render `lib.rs` draw path, physics `body_activity` + counter tests, `Cargo.toml`; read unchanged `timing.rs` from worktree; traced patch hunks for present/acquire, fence wait, submission counters, diagnostics gating, capture completion/failure paths. No execution; explicitly NOT RUN.
- **Issues & Friction**: `read` truncates large files (used offset reads + targeted greps for line numbers); no shell/git available so base-vs-candidate comparison relied on patch context (OUT_OF_DATE handling confirmed pre-existing that way); worktree already contains candidate changes so it could not serve as base.
- **Decisions & Rationale**: kept all 4 findings as doc-vs-code mismatches (the review's explicit ask), each with trigger/consequence/uncertainty/check; did not propose code rewrites or optimization work; treated worker test/clippy/fmt claims as unverified (NOT RUN) and made no causal speed claim; treated deferred streaming/collision/edit/queue counters as explicitly out of scope per doc.
- **Solutions Applied**: none (read-only leaf; no edits).
- **Insights**: the slice's honest core (epoch-keyed completion identity, empty-not-zero, elapsed-not-work streaming label, queue-span-not-execution GPU label, capture-gated clocks) is sound; the residual risks are all at definition boundaries — what counts as "completed save", "successful present", "same span", and "unavailable source". Fixing those four definitions is what makes the capture join-safe for P02. Handoff file `docs/performance/logs/muse-p01-behavior.md` left for the lead; independent Muse/Gemini review, Android build, graphics smoke, device overhead/repeatability, and validator lane integration remain lead-owned and NOT RUN here.

