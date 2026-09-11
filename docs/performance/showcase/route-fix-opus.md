# Showcase route fix — engineering log (bounded leaf session)

Scope owned: `crates/matterweave-detail/src/showcase.rs`,
`crates/matterweave-detail/tests/showcase.rs`, this log. Physics and
`crates/matterweave-physics/tests/showcase_traversal.rs` were read, never edited.

Target dir `/mnt/bench/matterweave-dev/performance/completion-02/route-fix-target`,
`CARGO_BUILD_JOBS=2`. Raw logs:
`/mnt/bench/matterweave-dev/performance/completion-02/route-fix-logs/`.

## Status at hand-off — NOT COMPLETE

`build_showcase` currently returns
`BudgetExceeded("elevated spine anchor unreachable on the walking grid")`.
The elevated spine's first anchor (cell 344,247 ≈ x 86.1, z 61.9) cannot reach
any later elevated anchor on the walking grid. Therefore:

- `matterweave-physics` traversal tests: **NOT RUN since the last source change**
  (last measured run below was on an earlier revision).
- `matterweave-detail` 19 showcase tests: **NOT RUN since the last source change**;
  they will fail while generation returns the error above.

Nothing here is claimed as passing. No device or performance measurement was made.

## Diagnosis (measured, not assumed)

The original route generator snapped each densified sample to the nearest
standable column independently. Standing points prove nothing about walking:
the actual character controller stalled between waypoints separated by walls of
0.5 m (ground point 12) and 1.9 m (elevated point 8).

Root cause in the source landform, `terrain_height_m`: the eastern terraces
quantise height to 3 m steps (`stepped = (h/3).floor()*3 + 0.4`, blended at
`terrace_t * 0.8`), producing risers up to ~2.4 m. No continuous walking route
to the viewpoint exists on that terrain at any step height a character should
have. Ash-ridge legs additionally hold sustained natural slopes above 1.0 m/m.

## What was built

1. `NavGrid` — derived walking connectivity at the terrain's own 25 cm column
   resolution. Connectivity is a property of *edges*: adjacent columns connect
   only when both are standable and the rise is ≤ `NAV_STEP_CELLS` (1 cell,
   0.25 m — inside the lead's accepted 0.30 m autostep). Bounded A*
   (8-connected, diagonals require both orthogonals) plus string-pulling with a
   straight-line walk test, then resampling at `ROUTE_STEP_M`. Terrain stays
   authoritative; the grid is derived and rebuilt per generation.
2. Trail corridors graded into the landform — the same technique the map already
   uses for landmark shelves (`flatten`): sample natural ground along each
   authored spine at 0.5 m, moving-average it, gradient-limit it to
   `TRAIL_MAX_GRADE`, then blend it back over a 1.6 m / 3.2 m verge.
3. Crossing agreement: the ground loop crosses itself and the elevated spur, so
   the per-corridor profiles are relaxed against each other and re-gradient-limited
   (`TRAIL_RELAX_ROUNDS`), and `Trails::grade` blends corridors by weight rather
   than choosing the nearest.
4. `SHOWCASE_GENERATOR_VERSION` bumped 2 → 3 (source layout changed).
5. Trail profile cache bounded to `TRAIL_CACHE_SEEDS = 8` seeds, LRU by use;
   profiles are built once per seed, never per column.
6. Unreachable spine anchors are now a hard generation error. Skipping them
   silently shortened the route and was rejected as acceptance.

## Failed experiments (in order, all measured)

| # | Change | Result |
|---|--------|--------|
| 1 | Hard shoulder-room rule: forbid cells with taller terrain within 2 cells | Every slope severed; A* failed from the first anchor for all 130 ground anchors. A 0.85 gradient already rises more than one cell across the capsule width. Replaced by a cost penalty. |
| 2 | Dry-land-only nav | Creek split the map; 40 anchor pairs unreachable. Added wading ≤ 0.5 m at heavy cost (`NAV_WADE_PENALTY`). |
| 3 | 1-cell (0.25 m) max step, ungraded terrain | Terraces impassable, elevated route unreachable. |
| 4 | 2-cell (0.5 m) max step, ungraded terrain | Route generated (ground 1366 m / 228 s) but traversal stalled: `ground point=186 target=[42.375, 20.5, 46.625] eye=[42.5, 21.894, 47.263] sim_s=81.4`. Narrow treads trap the capsule. |
| 5 | Corridor smoothing only (moving average, no gradient limit) | 15 anchor pairs still unreachable: smoothing shortens a riser but cannot bound a sustained slope. Added Lipschitz gradient limiting. |
| 6 | Independent per-corridor grading | Multi-metre steps where corridors cross; spine columns measured at slope 1.5–7.0. Added the joint relaxation. |
| 7 | `NAV_MAX_SLOPE` 0.45 with `TRAIL_MAX_GRADE` 0.40 | Elevated corridor isolated (10 unreachable pairs); lateral verge slope exceeds the along-track grade. |
| 8 | `NAV_MAX_SLOPE` 0.60, `TRAIL_MAX_GRADE` 0.30, weighted corridor blend | Centre-line slope diagnostic clean (no sample > 0.55 along the elevated spine), but the elevated start cell remains isolated on the walking grid. **This is the open blocker.** |

Best measured traversal state (experiment 4 revision, autostep 0.55 provided at
the time, now superseded by the lead's accepted 0.30):

```
TRAVERSAL PASS elevated=true points=37 sim_seconds=11.23 host_seconds=1.523
ground_loop_is_walkable FAILED: stalled point=186 sim_s=81.4
```
Raw: `completion-02/route-fix-logs/traversal-01.log`.

## Commands used

```
export CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/performance/completion-02/route-fix-target
export CARGO_BUILD_JOBS=2
cargo build -p matterweave-detail
cargo test -p matterweave-physics --test showcase_traversal -- --ignored --nocapture --test-threads=2
cargo test -p matterweave-detail --test showcase
```

## Requirements status

- 10 species, ≥4,000 plants, ≥2 M flora cells, ≥20 M expanded cells, 128 m world,
  overhangs and landmarks: untouched by this change — `place_flora`, the cavity
  set and the content gates are unmodified. **Not re-verified** (generation
  currently errors before the gates run).
- 3–5 min ground walk: last generated ground route measured 1,402 m ≈ 234 s at
  6 m/s, inside the window. Not valid for the current revision.
- Full spine / terminal viewpoint obligation: enforced as an error rather than
  satisfied. The elevated spur does not currently reach the viewpoint.

## Next actions for the lead

1. The single open blocker is the elevated corridor's connection at its junction
   with the ground loop near (86, 62). The centre line is graded and its slope
   samples are clean, so the isolation is a *grid* property: inspect
   `NavGrid::build` filters at that junction (slope from the 0.25 m central
   difference across the verge, `overhung` from the east-terrace cavity at
   (100, 17, 60), and the wade depth cap) rather than the profile.
2. If the junction cannot be opened, widen `TRAIL_HALF_WIDTH_M` locally at spine
   junctions, or lower `TRAIL_MAX_GRADE`, and re-measure both traversals.
3. Re-run the 19 detail showcase tests; several assert the old route properties
   (dry-land route points, route length) and may need honest updates.

## Risks

- The corridor grading cuts and fills the landform inside a 3.2 m band. Cut depth
  was not measured; a deep trench across a terrace is a visual regression risk.
- Generation cost grew: corridor profiles add ~3,150 base-height evaluations per
  seed plus an O(n²) relaxation over ~3,200 samples, both once per seed.
- The 19 detail tests were last exercised before the final source revision.

## Lead correction after worker handoff (2026-09-08)

Worker commit `039ad6f` was retained as a failed experiment, not integrated alone.
The lead completed the source correction against the accepted 0.30m Rapier step:

- Admit the source classifier's 0.85 quantized slope; gentle diagonal 25cm ramps can
  report 0.707. Require shoulder clearance rather than assigning an ineffective cost.
- Flood-fill actual walking edges from the entrance before snapping anchors so an
  isolated terrace cannot be selected. Missing as well as disconnected anchors
  now contribute to a hard generation error.
- Admit solid cave roofs as walking support. The previous overhang exclusion
  isolated six western anchors despite their real authoritative top cells.
- Keep the first two authored circuits covering the landmarks; the four-circuit
  trial took 408.58 simulation seconds. A subsequent 257.58-second trial silently
  missed anchors and was rejected. With all retained anchors resolved, both final
  routes pass: ground 632 waypoints/197.46667 simulated seconds, elevated 37/11.35.
- Fix the authored corridor elevation profile to the canonical seed. Terrain and
  flora still vary with seed, while path grades behave like other authored
  landmarks. This also replaces a per-column locked seed cache with one immutable
  profile. Both canonical and adjacent-seed source gates pass.
- Route tests now permit the declared <=0.5m wading while retaining physical
  footing/clearance checks. Water coherence samples are denser (17/19 cell strides
  instead of 29/31), retaining the same minimum sampled water count. No content or
  mesh budget threshold was lowered. Removed dead experimental penalty branches.

Executed: all 19 showcase source tests PASS (`lead-authored-profile-gates.log`),
actual two-route traversal PASS (`lead-final-walk.log`,29.15 host seconds), and
strict detail/all-target Clippy PASS (`lead-final-clippy.log`). An initial command
used the nonexistent test target `showcase_walk`; it was corrected to the actual
`showcase_traversal` target. Host timings are not phone performance evidence.

Independent review, integration rerun, source manifest, Android build and device
route/visual acceptance are owned by the lead and remain pending at this commit.
