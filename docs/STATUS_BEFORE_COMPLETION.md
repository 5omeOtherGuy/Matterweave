# Prior campaign handoff

Archived before the completion continuation; historical claims, not current runtime state.

## Next campaign: performance and dense alien showcase

Execution has started in the separate `codex/performance-p00` checkout from
`7556204`; see the [sole campaign board](performance/board.json) and
[P00 execution evidence](performance/p00.md). The existing [P00–P07 tasks](PERFORMANCE_TASKS.md)
and mandatory [showcase](SHOWCASE.md) remain the scope, not a replacement plan.

- P00: lead-written compact board and stateless patch-handoff validator implemented.
  Real Pi cancellation retained partial work; replacement rejects stale attempts.
  Independent Muse/Gemini reviews preceded lead inspection; 31 host validator tests
  pass after a reproduced artifact-type correction. Six exact route selection checks
  pass; selection alone is not model task qualification.
- P01: corrected typed CSV v2 code slice and profile validator accepted after
  independent Muse/Gemini reviews and lead verification. Python checks:31 handoff,
  65 conditions and130 profile tests. Real Android captures include1000 rows/epoch1
  and581 rows/epochs1–3; host replay fixture has10 tests. Full app/physics replay,
  some edit/streaming/collision counters and full instrumentation overhead remain
  incomplete. [Three short recorder pairs](evidence/2026-09-08-p01-overhead.md)
  now completed: ON mean-interval differences +0.032/+0.700/+0.673%, thermal0 and
  no unsupported SF gaps. Process CPU is a whole-process rate, not per-frame work.
- P02: CPU interpolated-geometry cache and transactional GPU upload invalidation
  implemented. Eight geometry and three upload-state tests constrain motion,
  sleeping rotations, wake/fracture/restore, empty geometry and renderer recreation.
  Combined native candidate `cc46873`:180 workspace tests, Clippy/fmt pass;
  90-frame normal Vulkan lifecycle smoke passes. Serial independent Muse/Gemini
  reviews/corrections completed and source is accepted for device evaluation.
  Real phone normal-mode2190 rows/epochs1–2 include sleeping cache hits and a
  resume upload without CPU rebuild. Matched APK comparison558274 stopped before
  capture: phone was colder than its old reference, not within matching limits.
  Establish a fresh cold reference; no qualified optimization win is claimed.
- P03: reviewed foundation and flora source merged through PR6 (`524f29eb`) and
  PR7 (`c71306d`), with host/Android/docs CI passing. Frozen flora64661cf/267c2c8:
  84 plants/six types,52686 expanded flora cells. An isolated opt-in
  [native gallery](performance/p03/native-gallery.md) now supports `flora source`,
  caches once and preserves the user-world path by construction. Coarse flora LODs
  are rejected. Actual phone flora rendered85 instances/120582 triangles and
  survived HOME/resume (2194 captured rows/epochs1–2); world bytes unchanged.
  Close-range anatomy, water/lighting, traversal/collision and cost/peak-memory
  acceptance remain open. Desktop has the
  nonoverlapping finest-source collision-adapter lane; lead owns app/phone/merge.
- Device: current save backed up and installed baseline APK hash verified. Owner
  enabled wireless debugging and unplugged USB; charging is confirmed off. Idle
  cooling observations are not app performance. First capture was rejected for a
  foreground/surface-identity problem. A [corrected short baseline](evidence/2026-09-08-performance-baseline.md)
  completed: 5,477 co-observed intervals, median16.583 ms, p95/p99≈33.17 ms;
  all eight sampled thermal statuses0. One baseline is not an optimization comparison
  or overhead qualification. User save restored; lead retains device/settings ownership.
- Owner authorized up to $9.99 existing Hy4 credit, capped by actual $9.985349664
  remaining. No key-level limit is configured; enforceable no-top-up execution remains
  pending. No purchases, top-ups or paid fallback authorized.

No mobile optimization win or complete native showcase is shipped. Next: finish
matched frozen-APK P02 comparisons and native flora quality/collision/cost gates. A separate `codex/p02-wsi-probe` diagnostic based on
P01 (`eb045e5`) logs16 captured draws without changing rendering policy. This is
not a performance comparator. Own-app simpleperf was denied by Android security;
no suggested security-property change or escalation was applied. First WSI trial's
post-reinstall save verification failed; explicit stop plus synchronous shell-write
restoration produced two byte-identical reads of the original. Its failed trial/log
is retained; corrected trial02 restored APK/save successfully. The subsequent
native functional trial also restored the save. Paired supervisor558274 subsequently
stopped after31 rejected observations (battery26.3°C/skin26.078°C versus old
31.3°C/31.483°C reference); no comparison capture launched. Final original-save
hash verified. Phone is stopped on the P01 baseline APK; no phone supervisor remains. Android build initially reused stale physics
metadata from the older probe's shared target. Scoped Android workspace-package
cleanup (no active build) and fresh rebuild passed; avoid cross-worktree targets.
Remaining P01/P04–P07 and full-map/water/lighting/density gates are not closed.
Raw overhead artifacts remain on `/mnt/bench`; remote artifact publication remains
an explicit gap. Source commits/repro commands and small summaries are durable here.

