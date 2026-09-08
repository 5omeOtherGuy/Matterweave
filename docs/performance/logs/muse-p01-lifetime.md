# P01 instrumentation candidate review (read-only)

Scope: `/mnt/bench/matterweave-dev/performance/run-01/p01-candidate-01` (frozen). Changed files read from candidate only. One unchanged supporting file read from worktree: `crates/matterweave-render/src/timing.rs` under `/mnt/bench/matterweave-dev/worktrees/performance-p00`. No execution checks — **NOT RUN**. Worker claims (tests/fmt/clippy pass) verified by source/log inspection only, not by re-running. No causal speed claim made or endorsed.

Contract checked: instrumentation-only, opt-in `profile-frames.txt` semantics, exact integer identities + epoch, empty-not-zero, CPU busy vs wall, stage/wait boundaries, physics active/sleeping/non-simulated excluding character, no GPU-compositor join, `measurement-v2.md` vs code, v0.3 wall/GPU-total overclaim guard, deferred counters explicit.

Verdict: solid slice, docs largely honest. 4 bounded findings below. No optimization implementation.

## Findings (4)

### F1 — `main_cpu_busy_ms` window strictly larger than `main_wall_ms`, doc says "same span"
- Files: `apps/explorer/src/lib.rs:944-945`, `1069`, `1071-1088`; `docs/performance/measurement-v2.md` CPU-busy section.
- Trigger: every captured frame. `cpu_busy_begin` is taken *before* `now = Instant::now()` (line 944 vs 945); `self.cpu_ms = now.elapsed()` stops at line 1069, but the second `thread_cpu_time()` is taken at lines 1085-1088, *after* `gpu_timings()` + `draw_diagnostics()` + `shadow_caster_meshes()` reads (lines ~1073-1084).
- Consequence: busy interval ⊃ wall interval. A frame can report `main_cpu_busy_ms > main_wall_ms` by the cost of those intervening calls, weakening the "wall without matching busy discriminates waiting" discriminator and any busy/wall ratio.
- Evidence: read order in `draw()` as above; doc states busy is "the delta … across the draw attempt" / "over the same span".
- Uncertainty: magnitude expected small on host (a few syscalls + Vulkan query read); not measured here (NOT RUN); whether the extra window ever dominates on phone unknown.
- Discriminating check (read-only): move the second `thread_cpu_time()` immediately adjacent to the `self.cpu_ms = now.elapsed()` line with no intervening renderer queries, or amend `measurement-v2.md` to define the two windows exactly (busy_start < wall_start < wall_end < busy_end).

### F2 — `voxel_bodies_not_simulated` conflates below-world clamp with residency/distance policy
- Files: `crates/matterweave-physics/src/lib.rs:191-192`, `317-329`; `docs/performance/measurement-v2.md` work-counters table.
- Trigger: any body clamped at/below y=-32. `fixed_step` does `body.set_enabled(supported && dx<40 && dz<40 && y > -32.0)`; `body_activity()` counts all `!is_enabled()` as `not_simulated`.
- Consequence: a fallen/clamped body inflates `not_simulated`, which the doc defines as "disabled by the residency/distance policy". A consumer attributing `not_simulated` to streaming policy misattributes world-boundary handling.
- Evidence: enable predicate includes `body.translation().y > -32.0` alongside `supported`/distance; counter has only one disabled bucket; `total == active+sleeping+not_simulated` still holds so no arithmetic break.
- Uncertainty: rare path (requires fall-through); NOT RUN; Rapier `is_enabled`/`is_sleeping` semantics taken from source + counter tests, not device behavior.
- Discriminating check: read `fixed_step` enable block vs doc definition; decide to either document "not_simulated = any `set_enabled(false)`, including below-world clamp" or split the bucket.

### F3 — "No extra clock readings when disabled" overstates opt-in
- Files: `apps/explorer/src/lib.rs:945`, `957`, `1016` (unconditional `Instant::now` for `now`/`stream_begin`/`mesh_begin`); `save()` timer + `TimestampQueries::begin/recorded_at` unconditional in renderer path; `docs/performance/measurement-v2.md:79-81`.
- Trigger: every uninstrumented frame. Doc correctly lists `draw_interval/main/stream_request/mesh_sync` as always-taken, then concludes "so an uninstrumented run takes no extra clock readings".
- Consequence: the conclusion is false as written — an uninstrumented run still pays ~3-4 `Instant::now` + elapsed per draw plus the pre-existing GPU timestamp `recorded_at` write/read. Phone overhead/repeatability analysis must still budget these; they cannot be assumed zero. This does not invalidate the opt-in gating of the *new* diagnostics (those use `capturing.then(...)` / `diagnostics_enabled.then(...)` correctly), only the summary sentence.
- Evidence: unconditional bindings above vs gated `physics_begin` (`:979`) / `render_begin` (`:1030`) / `cpu_busy_begin` (`:944`, lazy via `then`); `timing.rs` `begin()` sets `recorded_at = Some(Instant::now())` on every submit regardless of capture (unchanged file, read in worktree).
- Uncertainty: cost likely negligible vs frame, but phone clock cost not measured (NOT RUN, explicitly deferred to campaign lead).
- Discriminating check: count `Instant::now`/`elapsed` executions with `capturing == false` by reading `draw()` + `sync_render_meshes(capturing)` + `timing.rs::begin/read_completed`; reword doc to "no *additional* capture-only readings; the N baseline clocks remain".

### F4 — Retry rows can carry a stale `shadow_caster_meshes` from the previous attempt
- Files: `apps/explorer/src/lib.rs:1080-1084`; `crates/matterweave-render/src/lib.rs:1329-1340` (early `Retry` before `shadow.update/record`); `docs/performance/measurement-v2.md` counters table.
- Trigger: early retry (zero-size surface path; swapchain-recreate failure; acquire `OUT_OF_DATE`) — all return before `shadow.record`. Explorer still reads `renderer.shadow_caster_meshes()` and writes it into the retry row.
- Consequence: a retry row that "produced no submission" duplicates the prior attempt's caster count. A consumer summing/averaging `shadow_caster_meshes` over rows overcounts; time-series alignment implies shadow work in an attempt that did none. Doc says "submitted to the most recent shadow pass", which is literally true but row-scoped reading implies "this attempt".
- Evidence: renderer early-return precedes shadow recording; explorer `shadow_casters` read is unconditional post-draw (not gated on `Presented`); `submitted_gpu_frame_id` is correctly empty on retries but `shadow_caster_meshes` is not.
- Uncertainty: frequency depends on resize/suspend path; NOT RUN; `Shadow::caster_meshes` reset semantics taken from field read, not from executing a retry.
- Discriminating check: read `Shadow::update/record` for reset behavior; either write empty `shadow_caster_meshes` on rows where `submitted_gpu_frame_id` is empty, or document "retry rows repeat the most recent pass value; join shadow cost only on rows with a submission".

## What was verified as correct (no finding)
- Identities stay integer-typed end to end (`metrics::FrameRow` u64/u32, `write_id/write_count` decimal, 2^53 test), `draw_attempt_id` increments per renderer-reaching attempt, `presented_count` only on successful present, retry/OOM rows visible without invented submissions, `GpuCompletionTracker` keyed on `(epoch,id)`, v1 files preserved via `frame-profile-v2-` + `create_new`, malformed requests retained.
- Fence/acquire/present measured at true call boundaries in renderer (`fence_begin` before `commands.wait`, acquire clock around `acquire_next_image`, present clock around `queue_present`), `present_ms` documented as queueing-only, GPU span documented as queue-timestamp span including stalls (matches `timing.rs` TOP_OF_PIPE→BOTTOM_OF_PIPE), no scanout/compositor join.
- Physics counting excludes character (iterates `objects` only), `total==active+sleeping+not_simulated` tested; character collider toggling in `step_objects` touches colliders, not the counted set.
- Missing-as-empty enforced (`write_ms` rejects negative/NaN/inf; clock failure/backwards/out-of-range → `None`; non-Linux/Android → `None`).
- Deferred streaming/collision/edit/queue/alloc/phone-overhead explicitly listed as not measured; no GPU-pure-execution overclaim in the new slice.

## Engineering log
- Actions Taken: read frozen `sha256.json`, `measurement-v2.md`, `metrics.rs`, physics `lib.rs` + counter tests, explorer `lib.rs` draw/mesh/save/epoch wiring, renderer `lib.rs` diagnostics + `draw()` boundaries (incl. offset continuation), worktree `timing.rs`, writer log `opus-p01-a1.md`; grepped for clock/diagnostics/epoch/enable lines to pin file:line refs. All reads only; NOT RUN (no tests, builds, or device commands).
- Issues & Friction: `read` truncation required offset reads; `grep` line numbers partially elided so pins rely on combined grep+offset reads; base-vs-candidate unconditional-clock delta could not be fully established read-only without the base file, so F3 is framed as doc-overclaim rather than regression claim.
- Decisions & Rationale: kept to ≤4 concrete, trigger→consequence findings with uncertainty + discriminating check; did not broaden into optimization or fix implementation per instructions; treated worker test evidence as log testimony, not verification.
- Solutions Applied: none (read-only leaf; no edits).
- Insights: the (`main_wall`, `main_cpu_busy`, `fence/acquire/present`) tuple is the right waiting-vs-work discriminator, but F1's window skew must be closed before ratios are trusted; the epoch-paired GPU join is the load-bearing correctness piece across suspend/resume and is correctly implemented; the unconditional-1 dynamic build/upload plus sleeping-body counters already frame the P02 hypothesis without measurement, as the writer log notes.

