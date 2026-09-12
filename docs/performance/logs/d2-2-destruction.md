# d2-2-destruction — engineering log (2026-09-13)

Slice: complete the destruction and persistence half of D2 on the host — fracture
with exact counts and per-piece mass/inertia, persistent constraints that hold
under load, save/load topology round-trips and leak-free lifecycle cycles — with
the destruction bounds declared. Base `c095047e895bd8608df1e99204062e1475edc058`,
branch `phase-a/d2-2-destruction`.

The graphics-integrated Android 64-piece run and the 20 reset/load cycles on the
device are lead-owned and are **not run** here.

## What changed

### Destruction

- `break_body` is unchanged in behaviour except for two named constants: the
  radial separation term is now `offset * FRACTURE_RADIAL_RATE` (`1.5` s⁻¹) and
  the speed clamp is `MAX_BODY_LINEAR_SPEED` (`128.0` m/s). The energy it adds is
  documented on the method. Initial velocities are `v + ω × offset` (rigid
  decomposition preserves angular velocity; this is a kinematic fact, not a
  conservation claim) plus the intentional outward radial term. Magnitude: the
  radial speed is `1.5 · |offset|`, at most ≈1.95 m/s for a unit voxel in the
  largest supported body, and the extra kinetic energy is
  `½·m·Σ(1.5·|oᵢ|)² ≈ 8.6 J` for a full `[6, 2, 2]` beam. Deleting it would
  silently change fracture behaviour, so it stays.
- Mass and inertia were already derived per body from the collider shape and
  density; the new tests pin that `ρ = 3 kg/m³` (`BODY_DENSITY`) and that a split
  neither loses mass nor inherits the parent's inertia.
- The 64-body cap check before mutation is untouched and now exercised at both
  exact boundaries (63 + 2-voxel split → 64 allowed; 64 + 2-voxel split refused
  with the snapshot unchanged).

### Constraints (R07)

- New `Physics::constrain(first, second)` welds two voxel bodies with a Rapier
  fixed joint at their current relative pose. `local_frame2` captures
  `pose2⁻¹ · pose1`, so the weld holds the bodies where they are instead of
  snapping their origins together; contact between the welded pair is disabled
  because the joint is the connection.
- Refusals (out-of-range index, self, duplicate pair, `MAX_CONSTRAINTS`) are
  decided before any mutation. `constraint_count` excludes the transient grab
  joint.
- Fracturing either endpoint retires that body's welds with it. The registry is
  pruned against `ImpulseJointSet::get` immediately after body removal, so a
  retired joint handle can never be re-adopted by a reused handle. Child pieces
  are not re-welded; surviving constraints are untouched.

### Persistence

- New `PhysicsSave { version, snapshot, constraints }` and
  `ConstraintSnapshot { first, second }`, with `Physics::save` / `Physics::load`.
  Constraint endpoints are indices into `snapshot.bodies`, so a loaded save
  rebuilds the same graph; the weld frame is re-derived from the saved poses.
  Held grab joints are transient and never saved.
- `load` validates the whole save (version, `MAX_CONSTRAINTS`, endpoint range,
  self-pairs, duplicate unordered pairs) before `restore` touches live state, so
  rejected saves are atomic.
- `PhysicsSnapshot` / `BodySnapshot` / `snapshot` / `restore` are byte-for-byte
  unchanged, so existing app code and existing JSON saves keep working. The app
  currently creates no persistent constraints; a host that does must use
  `save`/`load` to keep topology.
- New read-only accessors for tests and game logic: `body_mass`, `body_inertia`
  (kg·m² principal moments), `collider_count`, `constraint_count`.

### Declared bounds

| Bound | Value | Kind |
| --- | --- | --- |
| `MAX_BODIES` | 64 | retained voxel bodies |
| `MAX_VOXEL_DIM` | 6 | voxels per body axis |
| `MAX_VOXELS_PER_BODY` | 32 | voxels per body |
| `MAX_FRACTURE_PIECES` | 32 | bodies one call can create; a split adds at most 31 |
| `MAX_CONSTRAINTS` | 128 (= 2 × `MAX_BODIES`) | persistent welds |
| `FRACTURE_RADIAL_RATE` | 1.5 s⁻¹ | intentional fracture separation energy |
| `MAX_BODY_LINEAR_SPEED` | 128 m/s | body speed clamp (also restorable bound) |
| `MAX_BODY_ANGULAR_SPEED` | 64 rad/s | body spin clamp |
| `BODY_ACTIVITY_RANGE` | 40 m (private) | X/Z residency: disabled with state intact, never deleted |
| `BELOW_WORLD_FLOOR` | −32 m (private) | fallen bodies retained inactive at zero speed |
| destruction queue / staging | **none** | synchronous fracture; refusal decided pre-mutation |

## Verification

| Check | Command | Result |
| --- | --- | --- |
| New destruction suite | `cargo test -p matterweave-physics --locked --test destruction` | PASS (9 passed) |
| Package tests | `cargo test -p matterweave-physics --locked` | PASS (88 passed, 2 ignored, 10 suites) |
| Workspace tests | `cargo test --workspace --locked` | PASS (545 passed, 3 ignored, 46 suites) |
| Format | `cargo fmt --all -- --check` | PASS |
| Lints | `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS (exit 0; 1 pre-existing vendored-winit warning capped by Cargo) |
| Docs | `python3 tools/check_docs.py` | PASS (198 files after this log) |
| Frozen regressions | `git diff -- crates/matterweave-physics/tests/detail_cadence.rs crates/matterweave-physics/tests/detail_structural_starvation.rs` | PASS (empty) |
| `Cargo.lock` | `git status --short` | PASS (unchanged; no new dependency) |

### Observed measurements (host, not device)

- **Weld under load**: 3-body tower (two 1 m cubes, one 9 kg `[6, 2, 2]` beam,
  1.5 m cantilever) stepped 600 fixed frames. Both welds held: center-distance
  drift 0.00053 m (weld 0) and ≈0.0000 m (weld 1), relative rotation drift
  0.0014 rad each, no body above 20 m/s or 20 rad/s. The unwelded control
  separated to 2.05 m against 1.43 m welded — 0.62 m beyond the 0.05 m failure
  tolerance, so the test discriminates rather than passing vacuously.
- **Mass/inertia**: a `[6, 2, 2]` beam is 9.000 kg with principal inertia
  `[1.5, 7.5, 7.5]` kg·m²; after splitting, 24 pieces sum to 9.000 kg and each
  piece is 0.375 kg with `[0.015625, 0.015625, 0.015625]` kg·m², matching its own
  0.5 m cube rather than the parent.
- **Exact counts**: the seeded scene (64 voxels in 6 bodies) ends at exactly
  `MAX_BODIES = 64` unit pieces, and two independent runs are bit-identical
  (`snapshot()` equality). The cap boundary allows 63 + 2-voxel → 64 and refuses
  64 + 2-voxel atomically.
- **Persistence tolerance**: poses, velocities and quaternion components
  round-trip through JSON and `load` within 1e-5 (the test's stated tolerance);
  two instances loaded from the same save track each other within 1e-4 after
  120 fixed steps, and constraint topology is exactly equal.
- **Lifecycle**: 20 cycles of fracture → step → flying-camera move → `load`
  returned to the baseline (3 bodies, 20 colliders, 2 constraints) every cycle
  and left the world steppable.

## Definition of done

| Criterion | Verification | Owner | Result |
| --- | --- | --- | --- |
| A body fractures into up to 64 pieces; the cap is enforced and the piece count is exact and reproducible for a seeded input | `seeded_fracture_piece_count_is_exact_and_reproducible`, `fracture_cap_allows_exactly_sixty_four_and_refuses_sixty_five` | worker | PASS |
| Total mass is conserved across a fracture, and each piece's inertia is consistent with its own shape and mass rather than inherited wholesale | `fracture_conserves_mass_and_rebuilds_per_piece_inertia` | worker | PASS |
| The intentional radial separation impulse is documented as such, with its magnitude stated; no silent energy change is introduced | `break_body` doc + `FRACTURE_RADIAL_RATE` doc; diff reviewed | worker | PASS |
| Constraints hold under load: a constrained assembly does not drift apart or explode over a bounded step count | `constrained_tower_holds_under_gravity_load` (600 frames + loose control) | worker | PASS |
| Persistence round-trips: save then load reproduces body count, poses, velocities and constraint topology within a stated tolerance | `save_and_load_round_trip_bodies_velocities_and_topology` (1e-5); `invalid_saves_are_rejected_without_mutating_live_state` for atomicity | worker | PASS |
| 20 consecutive lifecycle/reset/load cycles leave no leaked bodies, colliders or constraints, and counts return to the baseline | `twenty_lifecycle_reset_load_cycles_do_not_leak` | worker | PASS |
| Queue, staging and residency bounds for destruction are declared as named constants or documented limits, not left implicit | bounds table above; `MAX_FRACTURE_PIECES`, `MAX_CONSTRAINTS`, `BODY_ACTIVITY_RANGE`, `BELOW_WORLD_FLOOR`, `break_body` docs | worker | PASS |
| `cargo test --workspace --locked` passes | run above | worker | PASS |
| `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings` pass | run above | worker | PASS |
| Graphics-integrated Android 64-piece run with 20 reset/load cycles | device run | lead | NOT RUN |

## Handoff for the lead

Source commit: the commit on `phase-a/d2-2-destruction` carrying this log (base
`c095047e895bd8608df1e99204062e1475edc058`). No new dependency, no `Cargo.lock`
change, no excluded path touched, frozen regressions byte-identical.

Existing app seam (no app edit is needed and none was made):

- 64 pieces: `Action::Break` (`apps/explorer/src/lib.rs:506`) calls
  `Physics::break_body` (`:509`). The playground from `spawn_playground`
  (`:284`, `:480`) is 6 bodies / 64 voxels, so breaking all six reaches exactly
  64 unit pieces — the same shape as `playground_fully_fractures_to_body_budget`.
- Lifecycle cycles: `Action::ResetObjects` (`:470`) already does
  `snapshot()` → `restore()` → `spawn_playground()`; the host regression uses
  the same boundary plus `save`/`load`.
- New engine surface for a graphics run that wants welds: `Physics::constrain`,
  `Physics::save`/`load`, `PhysicsSave`, `ConstraintSnapshot`,
  `constraint_count`/`collider_count`/`body_mass`/`body_inertia`, and the new
  constants. The app creates no persistent constraints today, so wiring
  `save`/`load` into its save format is a host-side follow-up owned by the lead;
  `PhysicsSnapshot` is unchanged, so the current save format is not broken.

Follow-up outside the owned paths: `crates/matterweave-physics/README.md` was not
editable in this dispatch, so its public-surface and bounds tables do not yet
list `PhysicsSave`/`ConstraintSnapshot`, `constrain`/`save`/`load`,
`constraint_count`/`collider_count`/`body_mass`/`body_inertia`,
`MAX_FRACTURE_PIECES`, `MAX_CONSTRAINTS`, `FRACTURE_RADIAL_RATE`,
`MAX_BODY_LINEAR_SPEED` and `MAX_BODY_ANGULAR_SPEED`.

## Decisions & Rationale

- **`PhysicsSave` instead of extending `PhysicsSnapshot`.** `apps/explorer`
  constructs `PhysicsSnapshot { version, eye, bodies }` literals in excluded
  paths; adding a field would break workspace compilation. The composite save is
  additive, keeps every existing save valid, and makes the topology-preserving
  path explicit. `load` is the only path that restores constraints.
- **Fixed weld at the body frames.** `constrain` captures the current relative
  pose in `local_frame2`; the first probe of identity frames showed Rapier would
  otherwise drag the two origins together. Contact between the welded pair is
  disabled so internal contacts cannot fight the joint.
- **Index pairs as the whole topology.** The weld has no tunable parameters and
  its frame is a pure function of the saved poses, so persisting an index pair
  cannot lose information and cannot drift from the body list on load.
- **Welds retire with the fractured body.** Re-targeting a weld to a child piece
  would be an invented gameplay rule. Retiring is the conservative option and is
  pinned by `fracturing_a_constrained_body_retires_only_its_welds`.
- **`MAX_CONSTRAINTS = 2 × MAX_BODIES`.** A spanning tree over 64 bodies needs
  63 edges; 128 admits loops while keeping solver work and save validation
  bounded.
- **Named operational bounds, unchanged values.** `BODY_ACTIVITY_RANGE` and
  `BELOW_WORLD_FLOOR` replace the same literals in the residency policy, and
  `MAX_BODY_LINEAR_SPEED` / `MAX_BODY_ANGULAR_SPEED` replace the clamps, so the
  destruction/residency limits are declared instead of implicit.

## Insights

- Rigid-body decomposition already produces correct child mass and inertia
  because every body shares `BODY_DENSITY`; the only real fracture energy change
  is the radial term, which is why it needed explicit documentation rather than
  a "fix".
- Rapier fixed joints constrain local frames to coincide; persistence that saves
  only a body list and an index pair is complete exactly because the weld frame
  is re-derived from the saved poses.
- Fracture of a welded body is a topology event: `RigidBodySet::remove` retires
  the joints, but the registry must be pruned in the same call or a reused joint
  handle could re-adopt a stale entry.
