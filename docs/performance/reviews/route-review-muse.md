# Generator3 route review — route-review-muse

Frozen target `d58c85e`, integrated as `0d8225b`. Read-only Pi review.

No findings — scoped route change reviewed at final source; no proven correctness blockers.

Scope inspected (final source, no shell/diff):

- `crates/matterweave-detail/src/showcase.rs` (full, 1–2089): `Trails`/`TrailProfile`, `NavGrid`, `densify`/`waypoints`, `Terrain`, flora clearance, `build_showcase` gates.
- `crates/matterweave-detail/tests/showcase.rs` (actual path; prompt path `tests/showcase.rs` does not exist): 19 tests read.
- `crates/matterweave-physics/tests/showcase_traversal.rs`: continuous Rapier gate read.
- `docs/SHOWCASE.md`: loop 3–5 min, waterside + elevated, landmarks/camera routes, phone-traversable DoD.
- `docs/performance/showcase/route-fix-opus.md`: treated as historical only per instruction; final source governs.

Why no finding:

- Edge-connectivity model is sound: `NavGrid::step_ok` (both ends walkable + ≤1-cell rise), diagonal guard requiring both orthogonals, `straight_walk` line check, greedy `waypoints` merge + 2 m resample from authoritative `height_at_metres`. Unreachable/missing anchors are hard `BudgetExceeded`, not silent shortening.
- Source/derived agreement checked in code and covered by tests: `Terrain::generate` carves before reporting `top`; `column_material`/`build_terrain_voxels` flood carved-below-water cells; surface-vs-voxel, cavity-mouth lowering, and `roofed == manifest.overhang_columns` tests exist.
- Route/landmark obligations present: closed ground loop with 180–300 s gate, open elevated spur with terminal-waypoint + not-closed gates, both with footing + 1.8 m head-clearance checks; 10 landmarks (basin/creek×2/grove/clearing/viewpoint/saddle/3 cavities); seed/version/manifest lengths recorded.
- No avoidable serious generation cost seen: `trails()` is one `OnceLock` canonical profile; `Terrain::generate` is bounded 512²; `NavGrid` rebuild per route and 6-round relaxation are bounded and test-sharded via `OnceLock` map. No per-column lock/cache in final source.

Unrun checks (explicit):

- No shell run per instruction: no `git diff base3af4115`, no `cargo test`, no traversal rerun.
- Stated passes taken on trust: 19 detail tests incl. different-seed, ground 632 pts/197.47 sim-s + elevated 37 pts/11.35 sim-s Rapier gates.
- Still needed independently: phone traversal of both routes, Android build/device visual + perf evidence, integration rerun/manifest. Host sim seconds are not phone evidence.

Engineering log for lead:

- Actions: read final `showcase.rs`, both test suites, `SHOWCASE.md`, historical route-fix log as context only.
- Issues: none proven; prompt path `tests/showcase.rs` is actually `crates/matterweave-detail/tests/showcase.rs`.
- Decisions: do not report dirty accepted 0.30 m autostep physics dep; do not treat worker-log claims as current.
- Solutions: none required from this review.
- Insights: only residual risks (not findings): `spawn_eye` derives from `ROUTE_SPINE[0]` surface rather than snapped `route[0]`; canonical-seed trail profile blended into alternate-seed landform is intentional but unmeasured for cut depth; `NavGrid` hard 2-cell shoulder exclusion is load-bearing — keep traversal gate green before tuning.


Lead disposition: no new source correctness blocker. Astra waterside coverage
finding addressed by `60abc39`/`f887d67`: a recorded21-point prefix with
water-margin/endpoints checks, actual continuous traversal7.1166673simseconds.
Phone routes are a separate open gate.
