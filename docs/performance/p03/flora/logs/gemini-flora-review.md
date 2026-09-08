# gemini-flora-review — frozen candidate02

Read-only review; lead triage and repairs follow in execution_log.md.

Profile: {'provider': 'antigravity', 'model': 'gemini-3.8-flash-high', 'thinking': 'high'}; outcome completed; elapsed 206.11s.

### Candidate Issues

#### 1. `instance_support` discards sub-cell horizontal translation
- **Path / Line**: `crates/matterweave-detail/src/flora.rs:677-688` (and `support_height` lines 643-655)
- **Concrete Trigger**: Calling public `instance_support` with an off-center transform where `translation_m[0] != (cx + 0.5) * SCALE_TILE_M` or `translation_m[2] != (cz + 0.5) * SCALE_TILE_M`.
- **Consequence**: Horizontal position is snapped to tile cell centers (`cx = floor(wx / tile_cell)`), ignoring up to ±12.5 cm of actual world-space displacement. Contact columns are evaluated at the wrong world offset, potentially reporting `Some` when a foot overhangs water/drop-offs, or `None` when valid.
- **Source Evidence**: `flora.rs:682-684` truncates to `cx`/`cz`, then `support_height` reconstructs world X/Z as `(cx as f32 + 0.5) * tile_cell + rotated[0]`. In contrast, the test suite's independent check (`tests/flora.rs:438-439`) preserves the actual position via `draw.transform.translation_m[0] + rotated[0]`.
- **Uncertainty**: In `dense_tile`, instances are placed exactly on tile centers, so offset error is zero within the canonical fixture; this affects general callers of the public query.

#### 2. Manifest export `thresholds.met` omits declared species and type thresholds
- **Path / Line**: `crates/matterweave-detail/examples/flora_gallery.rs:226-232`
- **Concrete Trigger**: Generating a manifest for any scene where `vegetation_min` and `flora_cells_min` pass, but `types_min` (<5) or `per_species_min` (<4) fails.
- **Consequence**: False success claim in the exported `manifest.json`. The boolean `"met"` evaluates to `true` while 2 of the 4 declared thresholds in the same block are unsatisfied.
- **Source Evidence**: `flora_gallery.rs:231` defines `"met": vegetation >= DENSE_VEGETATION_MIN && flora_cells >= DENSE_FLORA_CELLS_MIN`, ignoring declared keys `types_min` (line 229) and `per_species_min` (line 230).
- **Uncertainty**: `dense_tile(FLORA_CANONICAL_SEED)` enforces all four bounds at generation time (`flora.rs:822, 847`), so the canonical artifact meets all four, but the exported summary predicate is defective.

#### 3. Manifest `distinct_flora_types` statically hardcodes constant length
- **Path / Line**: `crates/matterweave-detail/examples/flora_gallery.rs:214`
- **Concrete Trigger**: Generating `manifest.json` for a scene that contains fewer than all 6 species.
- **Consequence**: Inconsistent export metadata. `counts.distinct_flora_types` claims 6 even if `instances_by_prototype` contains fewer distinct flora types.
- **Source Evidence**: `flora_gallery.rs:214` writes `"distinct_flora_types": FLORA_SPECIES.len()`, unlike `dense_tile` (`flora.rs:841-846`) which derives distinct types dynamically from `scene.draws()`.
- **Uncertainty**: For canonical seed 20260908 all 6 species are instantiated, so the static value matches the canonical fixture count.

---

### Engineering Log

- **Actions Taken**: Performed independent read-only audit of `src/flora.rs`, `tests/flora.rs`, `examples/flora_gallery.rs`, additive palette definitions in `src/lib.rs`, and canonical manifest outputs. Tests NOT RUN.
- **Issues & Friction**: `instance_support` public helper conflated tile-column planning with generic metre-space support queries.
- **Decisions & Rationale**: Confirmed thresholds (seed 20260908: 84 instances ≥ 64, 50,798 flora cells ≥ 40,000, 6 types ≥ 5), explicit material policy isolation (fungus collision vs decorative leaves), bracket shelf reservation, and little-endian binary buffer layouts.
- **Solutions Applied**: Documented candidate issues with exact path/line, concrete triggers, consequences, source evidence, and uncertainties.
- **Insights**: Reusable query APIs must retain full continuous coordinates rather than adopting fixture-specific cell-snapping shortcuts.
