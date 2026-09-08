# Wetland save restoration validation

## Actions

2026-09-08; bounded leaf on base `4e899b3`, inheriting uncommitted Runtime,
save-selection and DetailScene work. No device access, replay changes, delegates,
or integration work. Lead owns final acceptance and STATUS/roadmap integration.

- Preserved the earlier workers' changes, inspected their briefs and implementation,
  and completed focused regression coverage.
- Runtime now selects journals using actual source edits and `Physics::restore`
  validation rather than shallow JSON validation alone.
- Added `DetailScene::fork_source`: clone authoritative prototypes and placements,
  not cached meshes. Apply edits to a transient fork and replace the live scene
  only after all edits succeed. Rejected private prototype identities, revisions,
  cells and allocations disappear with the fork.
- Corrected the inherited isolation regression: an unknown placement was rejected
  before any edit, so it did not exercise copy-on-write isolation. The corrected
  fixture has three placements sharing a prototype. Editing `a` creates a private
  prototype; editing still-shared `b` then fails against a deliberately existing
  `__instance_edit:b` prototype. Tests verify original identities/counts/cells,
  actual file selection of a valid recovery, and fresh recovery selection.
- Extended the actual full-map Runtime test: preserve an unknown-placement primary
  and an invalid-body newer recovery byte-for-byte, select an older valid recovery,
  and assert exact source edit, character eye and body snapshot round trips.
  `world.json` remains an untouched sentinel.

## Issues

- Earlier Opus work stopped after provider quota exhaustion (HTTP429). Its
  cell-by-cell undo approach was rejected because it retained private prototypes.
  Muse inherited the work and also stopped on HTTP429 before completing checks
  or committing. Their incomplete checks are not counted as passes here.
- The leaf's initial 300-second window interrupted final documentation/handoff;
  completion resumed with documentation/checks/commit only.
- One newly added pose-bound assertion failed: a point below a thin floor was
  collision-free, not buried. This was a fixture error, not proof that the bounded
  search failed. Replaced it with a four-metre solid column covering all nine
  candidate positions; the corrected suite passed.
- No verified pre-implementation RED run is available for the inherited fix.
  The passing legacy-undo counterexample demonstrates leakage; it is not a
  failing-test RED checkpoint. No coverage percentage was measured.

## Decisions

- Reuse `Physics::restore` instead of duplicating body validation rules. A separate
  empty probe world receives candidate bodies; it never becomes session physics.
  It may retain a rejected edit candidate's validated bodies until the next
  restore, which replaces them, or until the probe is dropped. No rejected bodies
  enter the live Runtime.
- Keep primary-file precedence. If invalid, visit recovery slots 128 down to 1,
  accepting the first restorable candidate. Otherwise select the first absent slot
  in 1 through 128. Never overwrite an invalid file during selection.
- Reuse the source scene rather than regenerate the procedural map per candidate.
  One fork exists at a time. Each scene's authoritative payload is bounded by
  32 MiB; instance records, maps and allocator overhead are additional, so this is
  not a total-memory bound. Primary plus at most 128 recovery candidates can be
  attempted, each with at most 4096 edits and 64 bodies and a 2 MiB input limit.
  Fresh-slot discovery may read up to another 128 files. No latency or mobile
  memory claim is made for the worst case.

## Solutions and verification

Commands used the exclusive target
`/mnt/bench/matterweave-dev/performance/completion-02/save-validation-target`
and `CARGO_BUILD_JOBS=2`.

| Command | Actual result |
| --- | --- |
| `cargo test -p matterweave-explorer --lib journal_tests -- --nocapture` | Initial corrected isolation tests: 5 passed. |
| `cargo test -p matterweave-explorer --lib wetland -- --nocapture` | First expanded run: 14 passed, 1 failed (invalid burial fixture), 1 ignored. After fixture correction and added assertions: 15 passed, 1 ignored, 45 filtered; 0.02 s test execution. |
| `cargo test -p matterweave-explorer --lib full_wetland_load_edit_collision_and_reload -- --ignored --nocapture` | Actual Runtime gate passed twice: 16.90 s, then 16.59 s after adding exact body-snapshot assertions. Final run: 1 passed, 60 filtered. The normally ignored gate was explicitly executed. |
| `cargo test -p matterweave-detail --lib` | 1 passed. This is the library unit target, not the crate's integration suites. |
| `cargo clippy -p matterweave-explorer -p matterweave-detail --all-targets -- -D warnings` | Passed twice. Existing dependency warning remains in `vendor/winit/src/platform_impl/linux/x11/ime/context.rs:161` (function-item integer cast); owned targets passed strict Clippy. |
| `rustfmt --edition 2021 apps/explorer/src/wetland.rs apps/explorer/src/wetland_state.rs crates/matterweave-detail/src/scene.rs` | Completed before final tests/Clippy. |

Final resumed-handoff checks passed:

- `rustfmt --edition 2021 --check apps/explorer/src/wetland.rs apps/explorer/src/wetland_state.rs crates/matterweave-detail/src/scene.rs`
- `python3 tools/check_docs.py`: 128 Markdown files, 272 local links, 15 ADRs,
  20 requirements.
- `git diff --check`: no whitespace errors. Diff hunk inspection confines source
  changes to imports, Runtime load validation, focused tests and the scene fork;
  no replay or render-loop hunks changed.

Passing suites were not rerun during the resumed handoff. The exclusive generated
Cargo target is removed after completed verification; no shared cache is cleaned.

## Insights and remaining limits

- A valid grounded eye is preserved exactly. Source-overlapping eyes are probed
  at the saved position and eight upward 0.125 m increments (maximum 1 m).
  If all fail, the same bounded search is attempted at the entrance: at most 18
  collision queries. Correction logs the original and replacement eye; horizontal
  movement occurs only through the explicit entrance fallback. Source edits and
  bodies are not silently discarded. Pose probes precede dynamic-body restoration
  so contact with a saved body does not itself force viewpoint relocation.
- **Collision construction remains after candidate selection.** A source journal
  can pass edit/body validation and subsequently fail `replace_detail_scene`.
  Such a failure does not currently select another recovery journal.
- **Both-pose-blocked limit:** if both saved and entrance poses remain blocked
  throughout their bounded lifts, Runtime returns an error after selection.
  It does not currently reject that journal and try another recovery slot.
  Thus this is not complete recovery for every possible Runtime failure.
- Pose correction and exhausted lift are tested against real small source/Rapier
  fixtures. Full Runtime tests cover valid pose/edit/body recovery, not a buried
  full-map viewpoint or entrance fallback. No phone, Android build, full-workspace
  suite, worst-case 128-candidate performance, or coverage run was performed here.
- Lead should assess whether collision-build and both-pose-blocked failures need
  candidate-level validation before integration; this leaf makes no final product
  acceptance claim.
