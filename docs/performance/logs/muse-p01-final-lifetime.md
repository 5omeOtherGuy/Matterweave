# P01 candidate02 bounded read-only review — NOT RUN, no device, no performance claim

Scope: frozen ` /mnt/bench/matterweave-dev/performance/run-01/p01-candidate-02`; changed source read only from candidate02; `correction.patch` (candidate01 → worktree) used to scope changed hunks; `sha256.json` pins files. No edits/shell/agents/credentials/device. No execution checks — explicitly NOT RUN. Worker `fmt/clippy/PASS`, `10 tests PASS`, `60 tests` claims not re-executed here.

Instrument contract checked: typed IDs + epoch, retry before OR after accepted submission, out-of-date `present` → `Retry` but preserves `submission id`, count only successful API presents not scanout, no waits moved/removed, `WaitTally` at actual sites + reset before mesh sync + separate render fence wait, CPU span inside wall reads, shadows empty if no submission, save attempts/failures + attempted wall reset per row, 37-column name/value test, physics category assertions, docs match missingness/clock-baseline/scope.

## Candidate findings (all investigated, none valid)

### C1 — `device_wait_idle` on recreate path not in either fence column — NOT VALID
- File:line: `crates/matterweave-render/src/lib.rs:1473` (`device_wait_idle` in `draw` recreate branch) vs tally sites `:1258,:1279,:1299,:1312` + render wait `:1431`.
- Trigger: swapchain `recreate` → full device idle before `Swapchain::new`.
- Consequence if valid: fence-wait sum would understate that one recreation frame.
- Evidence: idle call is present and unmodified (no wait removed); docs scope is explicitly `every Commands::wait call site` (`docs/performance/measurement-v2.md:~Fence wait scope`), and `device_wait_idle` is not a `Commands::wait`; both fence columns are defined only over upload/retain + in-draw fence waits.
- Uncertainty: cannot measure frequency/cost without device; recreation frames are exceptional by construction.
- Discriminating check (not run): capture a recreation frame and confirm row still records with both fence columns present (explicit zero vs missing) and docs scope unchanged. Lead owns device qualification.

### C2 — Out-of-date present keeps `submitted_frame_id` on a `Retry` row looks like invented submission — NOT VALID
- File:line: `crates/matterweave-render/src/lib.rs:1636-1660` (`submissions+=1`, `submitted_frame_id=Some`, then `OutOfDate => Retry` preserving identity); `apps/explorer/src/lib.rs:1057-1080, 1114` (maps `Presented`→presented-count+1, `Retry` no increment, records `submitted_gpu_frame_id` from diagnostics).
- Trigger: `queue_present` → `ERROR_OUT_OF_DATE_KHR` after accepted `queue_submit`.
- Consequence if valid: retry rows with submission ids would conflate attempts/submissions or over-count presents.
- Evidence: intended corrected semantics per contract: retry can occur after accepted submission; `presented_count` advances only on `FrameResult::Presented` (`:1071-1079`), `present_ms` still times the failed call, docs state retry-after-submit keeps real id (`measurement-v2.md:Identities`), `metrics.rs` retry test covers before-submit empty vs after-submit `Some(2)`.
- Uncertainty: no live swapchain OOD observed here.
- Discriminating check (not run): source inspection of `classify_present` + explorer mapping suffices; device run owned by Lead.

### C3 — `draw()` diagnostic clear could leak prior-frame upload waits — NOT VALID
- File:line: `crates/matterweave-render/src/lib.rs:1404` (`begin_frame_diagnostics`), `:1416` (`draw_diagnostics` synthesizes from `WaitTally`), `:1420-1428` (`draw` clears only `submitted/render_fence/acquire/present`, preserves via `..`), `apps/explorer/src/lib.rs:962` (reset before mesh sync).
- Trigger: multiple upload/retain waits in one frame, then draw.
- Consequence if valid: prior waits leak into next row or render wait double-counts.
- Evidence: `begin_frame_diagnostics` resets both `diagnostics` and `WaitTally(enabled)` before mesh sync; upload columns are never stored in `diagnostics` — `draw_diagnostics` injects `upload_waits.total()/count()`; `draw` preserves tally by not touching it; disabled tally takes no clock and reports `None` vs enabled-zero `Some(0)` with unit tests.
- Uncertainty: none on code path; timing overlap relies on adjacent wall/CPU reads (see C5 scope).
- Discriminating check (not run): existing `WaitTally` reset/accumulation unit tests cover leak/accumulate; no new check needed.

### C4 — Mutation-test wipe lost `body_activity` logic — NOT VALID
- File:line: `crates/matterweave-physics/src/lib.rs:181-200` (current `body_activity` loop).
- Trigger: worker `git checkout` briefly wiped attempt1 physics file, restored from frozen candidate01 hash + reapplied changes.
- Consequence if valid: physics category counts silently changed.
- Evidence: `correction.patch` physics hunk shows only doc-comment + `counter_tests` assertion expansion; counting loop (`enabled→sleeping→active`, character excluded, `not_simulated` = disabled) is byte-identical in structure to pre-change intent; current doc (`outside resident columns / distance limit / y<=-32 floor`) matches actual `not_simulated` bucket without distinguishing reasons. Current `body_activity` is intended unchanged apart from tests/docs per task statement.
- Uncertainty: full-file hash comparison to candidate01 not recomputed here (read-only review, no execution); relies on `sha256.json` + patch hunk scope.
- Discriminating check (not run, Lead-owned): `sha256`/diff source check of physics file against frozen candidate01 hash, plus existing `counter_tests` category-sum assertion.

### C5 — Conditions validator lenient on bool/NaN/duplicates/cached skin — NOT VALID (Gemini owns coverage; read-only cross-check only)
- File:line: `tools/performance/validate_conditions.py:52-54` limits, `:69` battery markers, `:129` battery, `:163` thermal, `validate_readiness`/`load_jsonl` size/JSON guards.
- Trigger: JSONL `elapsed_s` bool/NaN, duplicate keys/fields, cached skin substitution, simulated battery.
- Consequence if valid: false idle-readiness pass.
- Evidence: `_finite_number` rejects `bool` and non-finite; `json.loads(parse_constant=_reject_json_constant, object_pairs_hook=_reject_duplicate_keys)` rejects NaN/Infinity + dup keys; `_parse_scalar_field` rejects missing/duplicate/empty for all 4 power fields + `status==3` + HAL boundaries; skin taken ONLY from single `Current temperatures from HAL:` section terminated by required cooling-devices line, exactly one `skin` entry, cached never used; `updates stopped/test mode/simul` rejected; `>=5 samples, >=120s span, <=35s gaps, monotonic finite nonneg elapsed`, `battery<=1C/skin<=2C`, `1 MiB/128 KiB` enforced; result `idle_readiness_only`, CLI nonzero concise, API `ValueError`, input opened `rb` read-only stdlib-only.
- Uncertainty: 60-test/cooling-record claims not re-run; no synthetic timings accepted per task.
- Discriminating check (not run): Lead's timestamped cooling records + source checks; no new orchestration service or general schema library needed.

World fixtures (independent assumption check, not trusting counts): `crates/matterweave-core/tests/performance_replay.rs:88,138,216,301,322,343,375` + `tests/fixtures/performance_replay_v1.json` is host-only JSON tape over real public `generate/stream_around/set/save/load` APIs, `deny_unknown_fields`, pinned `fixture_version==1` + `GENERATOR_VERSION`, unbounded/unknown-op/missing-expected rejected, exact save-bytes replay + resave equality, negative-boundary eviction/reversal and noop guards; no prod API change and no app/phone-input or 64-body claim in file header. No finding.

## Engineering log
- Actions Taken: read `sha256.json`, full `correction.patch`, candidate02 `matterweave-render/lib.rs` diagnostics/draw/present chain, `explorer/lib.rs` frame/diagnostics/save/shadow seams, `explorer/metrics.rs` schema/CpuBusySpan/shadow-gate, `matterweave-physics/lib.rs` body-activity, `measurement-v2.md` identities/timings/fence-scope/CPU docs, `validate_conditions.py` + `performance_replay.rs` headers/validators (all read-only, bounded offsets, no execution).
- Issues & Friction: large render file required chunked reads; validator/replay line numbers from grep without running; physics restore relies on frozen-hash provenance owned by Lead.
- Decisions & Rationale: treated `device_wait_idle` as out-of-contract (not `Commands::wait`) rather than a missing wait; treated OOD-with-submission as corrected contract, not double-count; treated stored-vs-synthesized upload fields as no-leak by construction; deferred all device/overhead qualification to Lead.
- Solutions Applied: none (read-only leaf, no edits).
- Insights: fence-wait split (mesh-sync sum+count vs in-draw) plus explicit `Some(0)` vs `None` and `begin_frame` reset ordering are the load-bearing seams; shadow-gate on `submitted_frame_id` and save `attempts/failures` reset-per-row are correctly wired at `explorer/lib.rs:1106-1150`; docs correctly scope missingness, clock baseline (4 pre-existing readings + save timer + `timing.rs` still budgeted), and unmeasured streaming/collision/edit/queue + driver-internal waits.

