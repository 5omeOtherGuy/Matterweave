# map-gemini-finish

Frozen952d630; independent read-only candidate findings.

### Correctness Findings

#### Finding 1: Unfiltered flora scatter places collidable vegetation into carved cavity mouths, violating empty-cavity volume invariants
- **File / Line**: `crates/matterweave-detail/src/showcase.rs:1246-1250`
- **Trigger**: `place_flora` tests `if surface.water_depth_m > 0.0 || surface.slope > 0.85 || surface.overhung { continue; }`. In columns where a cavity intersects and lowers the terrain surface (i.e. `raw_top > top`), the cavity is an open mouth: all solid cells above the carved floor were removed, leaving no overhead solid roof. Consequently, `surface.overhung` evaluates to `false`. When classified as `DETAIL_SOIL` or `MOSS_TURF` with gentle slope, flora instances (such as stipe-bearing fungi or woody shrubs) are scattered directly onto the lowered floor.
- **Consequence**: The placed flora’s collidable voxels extend vertically into the column’s carved void (`(surface.top_cell + 1)..=phantom_top`). Any downstream query checking whether the carved cavity volume is clear of solid collision—such as `surface_query_reports_real_solid_in_cavity_intersected_columns` (`tests/showcase.rs:417-425`) via `assert!(!scene.is_collidable_world_metres([x, cy, z]))`—detects the plant's collidable structure and fails with `"solid voxel above the reported surface at {x},{z},{cy}"`.
- **Evidence**: 
  - `src/showcase.rs:444-447`: `overhung[idx]` checks only whether carved air exists *below* `carved_top`. In lowered cavity mouth columns, `carved_top` is the floor of the cavity, so `overhung` is `false`.
  - `src/showcase.rs:1246-1250`: `place_flora` filters only `surface.overhung`, not whether `surface.top_cell < raw_top` or whether `(x, z)` touches a cavity floor.
  - `tests/showcase.rs:411-426`: `surface_query_reports_real_solid_in_cavity_intersected_columns` scans every column where `phantom_top > surface.top_cell` and asserts that every cell between `surface.top_cell + 1` and `phantom_top` has no collidable voxels in the scene.
- **Uncertainty**: Low. Code paths and logic in `place_flora` and the failing test assertion are fully matched.

---

#### Finding 2: `route_clearance` evaluates discrete route points instead of polyline segments, allowing corridor intrusion between steps
- **File / Line**: `crates/matterweave-detail/src/showcase.rs:1179-1182`
- **Trigger**: `route_clearance` tests candidate plant origins `(x, z)` using `route.iter().any(|p| dist2(p[0], p[2], x, z) < limit)`. The route polyline is densified with step size `ROUTE_STEP_M = 2.0` metres (`src/showcase.rs:59`, `864`). 
- **Consequence**: A plant placed midway between two successive route waypoints (e.g. 1.0 m along the chord) at a lateral offset from the path can satisfy `dist2(p, plant) >= limit` for both discrete endpoints while being significantly closer to the actual walked line segment than `limit`. A plant whose source radius plus `ROUTE_PLAYER_CLEARANCE_M` exceeds the distance to the segment will have physical voxels encroaching directly into the walked path corridor between waypoints.
- **Evidence**:
  - `src/showcase.rs:1179-1182`: Clearance evaluates point-to-point distance against discrete vertices `p in route`, not segment-to-point distance `dist_to_segment(p[i], p[i+1], (x, z))`.
  - `tests/showcase.rs:624-635`: Acceptance test `plant_source_geometry_clears_both_routes_including_its_own_radius` tests the identical vertex-only loop `for point in route { assert!(d >= limit) }`, masking continuous corridor encroachment.
- **Uncertainty**: Low regarding geometric reality (midpoint chord slack on 2.0 m intervals allows lateral intrusion); moderate regarding whether gameplay navigation treats vertices as straight rails or allows small corridor deviations.

---

#### Finding 3: `source_radius_m` measures Chebyshev $L_\infty$ half-extent, underestimating diagonal Euclidean clearance by up to $\sqrt{2}$
- **File / Line**: `crates/matterweave-detail/src/showcase.rs:111-125`
- **Trigger**: `source_radius_m` extracts the local AABB cell bounds (`volume.cell_bounds()`) and returns `low.max(high)` across horizontal axes 0 and 2 (`[0usize, 2].into_iter().map(...).fold(0.0, f32::max)`). 
- **Consequence**: The returned radius is an axis-aligned box half-width rather than the maximum Euclidean distance to an occupied voxel. For prototypes with diagonal extent (e.g. wide caps or asymmetric shrub branches reaching towards $(+x, +z)$), the true distance to the furthest occupied cell corner is $\sqrt{x^2 + z^2}$, which exceeds $\max(|x|, |z|)$ by up to a factor of $\approx 1.414$. When rotated by quarter-turn yaw, diagonal voxels penetrate deeper into the circular radial clearance threshold than accounted for by `radii[species]`.
- **Evidence**:
  - `src/showcase.rs:120-124`: `[0usize, 2].into_iter().map(|axis| ...).fold(0.0f32, f32::max)` computes $\max(\Delta x, \Delta z)$ rather than $\max_{\text{cells}} \sqrt{x^2 + z^2}$.
  - `src/showcase.rs:1179-1180`: `route_clearance` uses Euclidean `dist2(p[0], p[2], x, z)` against `ROUTE_PLAYER_CLEARANCE_M + radius_m`, assuming `radius_m` bounds Euclidean reach in all directions.
- **Uncertainty**: Low on the mathematical divergence between $L_\infty$ and $L_2$; prototype voxel distributions determine the exact excess (0 cm for purely cross-shaped prototypes, up to several decimetres for diagonal corners of large caps/shrubs).

---

### Engineering Log

| Component | Status | Verification Summary | Risk / Notes | Action |
|:---|:---|:---|:---|:---|
| **Archetype Placement** | Incomplete | All 10 prototypes placed ($\ge 100$ instances each, 3 habitats + shallow sweep). | Cavity mouths receive flora that invalidate cavity clearance queries. | Filter cavity columns in `place_flora`. |
| **Route Clearance** | Flawed | Clearance check and corresponding test both evaluate only discrete polyline vertices. | Plants between 2 m waypoints or with diagonal voxels penetrate walking corridor. | Check segment-distance & Euclidean radius. |
| **Cavity / Surface Agreement** | Failing | `surface_query_reports_real_solid_in_cavity_intersected_columns` failed in suite. | Caused by collidable flora placed on lowered cavity mouth floors. | Prevent flora placement where `raw_top > top`. |
| **Overhang Column Accounting** | Consistent | Tile-local bitflags carry across bands; deep solid base bands placed strictly below cavities. | Roofed count in `Terrain::generate` and `build_terrain_voxels` match identical criteria. | Maintained as-is. |
| **Density & Meshing Preflights** | Sound | Expanded cells $> 20\text{M}$, flora $> 2\text{M}$, prototypes $< 33{,}288$ cells. | Aggregate mesh cache and source payload bounds safely respected. | Maintained as-is. |

---

### Coverage Gaps
1. **Segment-to-plant corridor testing**: `tests/showcase.rs` tests route clearance only at densified vertex points `p`, omitting point-to-segment distance checks along the continuous traversed path.
2. **Flora collision isolation in cavity unit tests**: `tests/showcase.rs:418-423` tests `scene.is_collidable_world_metres` across the entire scene rather than discriminating terrain instances (`i_shell_*`) from flora instances (`flora_*`).
3. **Cavity floor flora exclusion**: Unit tests assert zero collision in carved column air without any explicit unit test or generator constraint ensuring scatter routines avoid cavity floors.

## Lead verification

Diagonal source radius and between-vertex route intrusion reproduced by actual-scene regressions; corrected without reducing density gates. Cavity overlap reproduced at [97.375,16.125,58.125] as parasol cap material21, not terrain: corrected terrain-only assertion passes610 lowered columns and2856 overhang columns. No product requirement forbids fungi on cavity floors; retaining them is intentional. Claims of measured counts in a read-only reviewer summary are not execution evidence.
