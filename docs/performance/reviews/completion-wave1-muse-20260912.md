# Frozen wave 1 review — muse

Target `22070ca`, base `704bb4a`. Independent source review; unverified candidates require lead triage.

## Review log — frozen-source, read-only

**Scope:** `git diff 704bb4a..22070ca -- crates/matterweave-audio crates/matterweave-physics/tests/detail_cadence.rs tools/performance/qualify_measurements.py tools/performance/test_qualify_measurements.py` + source/contract docs only. No worker logs, review outputs, or narratives read.

### Actions Taken
- Read `crates/matterweave-audio/src/service.rs` (suspend/resume/recreate, queue, health), `mixer.rs` (command apply, render, ack), `command.rs`, `backend.rs`, `backend/mock.rs`, `backend/aaudio.rs`, `config.rs`, `error.rs`, `lib.rs`, `service/tests.rs`, `tests/acceptance.rs`, `tests/recovery.rs`, `examples/audio_diagnostic.rs`.
- Read `crates/matterweave-physics/tests/detail_cadence.rs` fully and `src/detail_cadence.rs` (gate, staged fallback, version/supersession/reset semantics).
- Read `tools/performance/qualify_measurements.py` + `test_qualify_measurements.py` fully and contract `docs/performance/measurement-qualification.md`, `docs/adr/0016-audio-service.md`.
- Cross-checked arithmetic (noise vs overhead separation, `range_over_median_percent`, `on_vs_off_percent`, alternation, per-run matching) against contract and historical-evidence regression tests.

### Issues & Friction
- No shell/git tooling in this leaf; base↔target diff itself not directly viewable. Classification pre-existing vs introduced is therefore inferred from change description + code, marked uncertain where applicable.
- Line numbers below are current-worktree readings.

### Decisions & Rationale
- Report only defects with concrete trigger + consequence + code proof. Physics cadence + most qualifier gates verified sound; not reported.
- 3 candidate findings (≤6 allowed). No filler.

### Solutions Applied
- None (read-only; no edits/builds/acceptance).

### Insights
- Audio suspend/pause-preservation/retry structure and qualifier per-run matching, alternation, thermal/power/sample fail-closed gates, and physics discriminating geometry (tall 0.5 m wall vs old climbable 0.25 m; interior-fill equal-AABB case; workerless staged gate) all check out against contracts.
- The three items below survived validation and warrant lead reproduction.

---

## Candidate findings

### F1 — Unload generation check never succeeds; voices never RT-silenced, pool reused while RT reads
- **File:line:** `crates/matterweave-audio/src/mixer.rs:158-159`, producer `crates/matterweave-audio/src/service.rs:364-376`
- **Trigger:** any `unregister_clip()` followed by a render. Service does `next_generation(clip.generation)` and pushes `UnloadClip{slot, generation=new}`; RT slot still holds `old`.
- **Consequence:** `rt_slot.generation == generation` (new) is always false → `else` branch: `rt_rejected_commands+=1`, slot stays `active`, voices on that clip never silenced, `voices_silenced` never increments. `ack_epoch` still advances (per-render), so `reclaim_acknowledged_ranges()` frees/reuses the PCM range while the RT voices still read it → wrong-audio/corruption window; also every unregister inflates the "expected zero" RT-rejection counter.
- **Proof/source:** `mixer.rs:158`: `if rt_slot.active && rt_slot.generation == generation { ...active=false; silence... } else { rt_rejected... }`; success branch never updates `rt_slot.generation` either. Contrast `Stop`/`SetGain` which correctly compare against the *existing* handle generation. No acceptance test asserts `voices_silenced>0` or post-unregister RT silence.
- **Uncertainty:** medium — intended protocol inferred (Unload should carry old gen, or set `generation=new` and deactivate when `!=`). Needs owner confirm.
- **Class:** pre-existing (outside described suspension/recreation/retry/diagnostic change; logic shape suggests older bug).

### F2 — Suspend device-failure compensation silently dropped when FIFO full → mixer/device desync
- **File:line:** `crates/matterweave-audio/src/service.rs:511-530`, specifically `:525` (`let _ = self.push(Command::Resume)`)
- **Trigger:** queue has exactly 1 vacant slot at `suspend()` entry (pre-check passes), `Suspend` fills it, then `backend.suspend()` fails. Compensation `Resume` push fails `CommandQueueFull`.
- **Consequence:** `Suspend` remains queued without its paired `Resume`; `suspend()` returns `Err` with `running` still true, but the next render applies `Suspend` while the device never paused → unintended sustained silence (mixer suspended, device running). `rejected_commands` increments inside `push`, but caller gets no `CommandQueueFull` signal and cannot distinguish/retry.
- **Proof/source:** pre-check `if vacant==0 return QueueFull`, then `push(Suspend).expect(...)`, then `if let Err(e)=backend.suspend(){ let _ = push(Resume); return Err(e); }`.
- **Uncertainty:** low on code path; medium on field frequency of `request_pause` failure under load.
- **Class:** uncertain — likely **introduced** with suspend-hardening/compensation; needs base diff confirm by lead.

### F3 — Zero-noise identical repeats mislabelled "exceeds same-build noise range"
- **File:line:** `tools/performance/qualify_measurements.py:527-539` (`resolution()`)
- **Trigger:** all repeats identical in both states (floor range `0`, `mean_on_vs_off_absolute` `0`); e.g. all-OFF `20.0` synthetic case extended to ON.
- **Consequence:** `verdict` = `"below..." if floor>0 and abs<=floor else "exceeds..."` → `(0,0)` reports **"exceeds same-build noise range"** with `abs_mean_over_noise_range=None`. A perfect-repeatability report reads as distinguishable overhead.
- **Proof/source:** ternary at `:537-539`; `floor==0` always takes `else`. Tests cover `floor=1.0` and `floor=0.3` cases only, never `floor==0` with `mean==0`.
- **Uncertainty:** low.
- **Class:** **introduced** (new tool in this change).

---

## Checks NOT RUN
- No builds, no `cargo test`, no `python3 -m unittest`, no device access, no APK/collector/analyzer runs — explicitly out of authority for this leaf.
- No reproduction of F1/F2/F3 executed; lead owns reproductions and final acceptance.
- No `git diff` command executed (no exec tool); pre-existing vs introduced for F1/F2 needs lead base-diff confirm.
- Current-build on-device noise/overhead qualification remains **NOT RUN** per contract status; historical `2026-09-08-p01-overhead.json` arithmetic regression taken from in-repo tests, not re-executed here.
