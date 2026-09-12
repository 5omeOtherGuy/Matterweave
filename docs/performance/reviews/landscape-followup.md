# Landscape follow-up acceptance — 2026-09-12

DeepSeek V4.1 Flash/max implemented fixtures (w_b429a845) and researched streaming
(w_547ae261). Gemini/high initial review w_bed68cc5 required visible split ownership,
edit visibility and actual near-detail oracle coverage. Final code review w_599c6d28
found a test assertion incorrectly rejecting material 0 (deletion); the final worker
source had already removed it. Lead imported that correction and ran all four example
tests successfully. The existing old != new assertion still enforces a real edit.

Independent DeepSeek test w_8deca922 ran the built example with software Vulkan and
validation, checked actual images and recorded source/binary identity. Default:
16 runs, zero failures; landscape: 24 runs, zero failures. New matched images have
zero over-tolerance color differences; edit patches are 245 and 24 pixels. Near-detail
oracle hits are 37/35/84/82. No tolerance was loosened. Host evidence only.

Final example source SHA256:
`530c0fff905780c56789f522f5c19da8b29b010e45315754c193d8ad6c38faab`.
Tested example binary SHA256:
`49cfbcae9a11de6107d4a3578682ee6cd41e2ba0a81418ff3e1156d838304b97`.
The tester recorded the earlier eeff870 test-only difference explicitly; the delivered
source now exactly matches the worker source. Lead commands in the delivery worktree:

- `cargo test --locked -p matterweave-render --example renderer_comparison`: 4 passed.
- `cargo clippy --locked -p matterweave-render --example renderer_comparison -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `python3 tools/check_docs.py`: passed before final handoff edits; rerun at commit.

Cargo commands used the task target under `/mnt/bench/matterweave-dev/microvoxel-roadmap/target-fixtures`.
Vendored winit emitted its existing function-cast warning; no dependency edit made.
Independent raw evidence and image script are under
`/mnt/bench/matterweave-dev/microvoxel-roadmap/independent-test-output/`.
The first tester dispatch w_5049acf0 was stopped because read-only tools lacked bash;
the exact session resumed with executable tools and artifact-only write scope.

The original orthographic top-row mismatch remains a retained regression case:
116 pixels on row 0 in two matched runs, raster covered/ray background. Reproduce
by reducing the final fixture back-wall range from `-2..=8` to `-2..=6`, retaining
its camera. The worker's projected-edge explanation is plausible, not independently
proven as a complete renderer diagnosis. Passing revised framing is not a renderer fix.

Research was accepted after lead repairs to f32 precision (0.488 mm at 4096 m),
legacy-vs-production streaming limits, persisted edit propagation and D2 test-path
ownership. The coarse derivation proposal is not adopted or implemented.

No Android result or M2–M6 closure is claimed. The other lead owns the phone and
D1/D2/D4 integration. The next device step is to execute the opt-in harness with
its Android Vulkan environment when that lead schedules it.

Gemini/high reference assessment review w_cc916d6e identified ambiguous eviction
wording and a possible new automatic-support-detachment gate. Lead clarified local
cache paging vs derived LOD, and made support detachment a later trial. The reviewer
understated existing destruction scope: D2 destruction/64-piece requirements remain
intact. This was an inference/scope review; primary-source verification was by lead.
