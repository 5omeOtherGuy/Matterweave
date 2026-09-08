## Engineering log — leaf review 577dff8 (delta since 1cb468e)

**Scope:** `crates/matterweave-render/src/async_indirect.rs`, `async_indirect_tests.rs`, `indirect.rs`, `lib.rs:upload_indirect/indirect_enabled/async_indirect_api_gate`, `shadow.rs:upload_indirect/update`, `examples/async_indirect.rs`, `apps/explorer/src/engine_check.rs`, `docs/performance/async-indirect.md`. No edits, no agents, no builds.

### Actions
- Read controller + queue state machine, identity keys, reset/shutdown paths.
- Read 25 focused tests + API gate + standalone CPU example + native Vulkan gate.
- Checked publication adapter (`Renderer::upload_indirect` → `Shadow::upload_indirect` → `Shadow::update` sun revalidation).

### Issues
- None — no concrete actionable bugs found in the bounded surface.

### Decisions
- `upload_indirect` checking only `source_valid(epoch+revision)` and not sun is **not** filed as a bug: `Shadow::update` (`shadow.rs:299-300`) disables GI when `indirect_sun != light_key(settings.sun)` before draw, so a wrong-sun upload self-heals to disabled, not wrong radiance. Tightening the upload check would be a speculative hardening, not a correctness fix.
- `world.clone()` under the queue lock, one-slice wasted work after `config.build()`, and no `shutdown` flag on worker panic are noted as efficiency/robustness observations only — no trigger with evidence, excluded per “concrete bugs only.”
- `move || run(&shared, &config)` lifetime is sound: `move` captures `config` by value into the `'static` closure; `&config` borrows from thread-owned storage.

### Solutions
- No code changes (read-only review).

### Insights
- Identities are coherent: `SourceKey{epoch,revision,sun}` in `source_key`, `valid_for`, `take_result`/`finish_job`/`retire_stale_running`, plus opaque `Arc<()>` generation surviving `u64::MAX` saturation (test-covered).
- Latest-wins, dedup-without-fork, foreign-poll-preserves, reset-accepts-same-key, shutdown-joins-after-one-slice properties all have direct deterministic or worker tests.
- Standalone example correctly varies `revision` (in-place roof edits, constant epoch) and compares every cell/face with different slicing — valid CPU regression, not a publication test.

## Review coverage
- Covered: AsyncIndirectLight correctness, source/replacement/reset identities, queue/thread lifetime, Vulkan publication adapter, standalone regression.
- Not covered (out of bound / owner-deferred): full GI/reflections, sustained efficiency, Android device run.

## Checks
- `cargo test -p matterweave-render async_indirect`: **NOT RUN** (lead owns integration; no build ops in leaf).
- `cargo clippy`: **NOT RUN**.
- Native `--async-engine-check` / phone run: **NOT RUN**.
- `python3 tools/check_docs.py`: **NOT RUN**.

**Result: PASS (no blocking bugs) — with above NOT RUN checks owned by lead.**

