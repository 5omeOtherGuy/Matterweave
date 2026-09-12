# Bounded coarse terrain derivation

S2 code slice on e5864bd, worker commit 0d5f0f6 plus lead provenance correction.
`World::coarse_tile(level, key)` derives a read-only 16³-material tile at level 1
or 2 (32/64 fine-cell extent). It retains source revision, seed, generator version
and streaming source mode. This API is experimental supporting code; production
streamed LOD, seam treatment and async publication are not integrated yet.

Any occupied fine cell makes its coarse aggregate occupied. Material comes from
the topmost occupied sample, then lowest x/z. Thin openings may fill visually;
coarse data never replaces authoritative fine world data or collision. Streaming
sources use explicit overrides (including empty tombstones), legacy saved-region
semantics and the existing generator. Eviction does not drop edits. Nonstreaming
worlds use stored cells only. Invalid levels/overflow/disjoint streaming requests
are errors; valid empty tiles are successful results. Partial domain tiles use air
outside the simulation domain. Source revision is deliberately conservative;
consumers also own world identity and compare source mode, because enabling
streaming changes derivation without incrementing revision.

Work is bounded to at most 262144 fine samples, up to 64 generated fine chunks
(256 KiB payload plus overhead) and a 4096-byte output payload at level 2. These
are structural bounds, not measured mobile performance. No world-size guarantee.

## Verification and review

- Worker DeepSeek V4.1 Flash/max w_0d2e3948: implementation and inline tests.
  Lead corrected local-to-world stride and scratch accounting, and restored a
  lead-owned board reservation the worker improperly reset. The worker logged it.
- Initial Gemini/high w_50467fee identified source/legacy/revision hazards. Lead
  rejected its below-y=-8 legacy test suggestion as nondiscriminating.
- Final Gemini/high w_4c0973aa: no actionable code findings at 0d5f0f6.
- Lead added source-mode provenance and a test for enable_streaming without revision
  change; focused Gemini/high w_9b15b59e found no issue.
- Independent DeepSeek w_a4e325af ran 26 original core tests and five public-API
  tests. It timed out after a 4/5 result: its hand-written expected negative-Y
  tile used y-key 0 for a cell at y=-8. Lead corrected the test key to -1 and reran:
  5/5 passed. Raw failing and corrected logs remain under the run root.
- Final core suite: **27/27 host and 27/27 physical OnePlus13 ARM64 passed**.
  [Device log](../evidence/2026-09-12-coarse-terrain/android-core-tests.log),
  [binary/source provenance](../evidence/2026-09-12-coarse-terrain/android-core-manifest.json).
  Scoped strict Clippy and formatting pass. APK visuals are a separate open gate.

Run root: `/mnt/bench/matterweave-dev/coarse-terrain/`. Host command:
`cargo test --locked -p matterweave-core --lib`. Cross-build that same test target
with `--target aarch64-linux-android --no-run`, pinned NDK28.2/API28 linker and
16KiB max-page-size; run the resulting test binary using a writable TMPDIR on the
phone. Exact binary hash and executed command are in the manifest.

The next workers consume these tiles through actual scaled mesh geometry and an
interactive Terrain Lab; those are not delivered by this core-only slice. The
owner transferred development/device integration to this lead after PRs25/26/27
merged at a91f7a0. Production detail/audio wiring and remaining engine gates also
remain open; merging adapter modules alone does not complete those integrations.
