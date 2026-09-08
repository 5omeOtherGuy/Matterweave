//! Full deterministic 128 m x 128 m alien fungal wetland showcase map.
//!
//! This is the *whole* showcase map required by `docs/SHOWCASE.md`, not another
//! representative 16 m tile. It is generated from one frozen seed
//! ([`SHOWCASE_SEED`]) and one authoritative 25 cm terrain source, plus reused
//! fine-scale flora prototypes at 6.25 cm / 12.5 cm.
//!
//! Structure and reuse
//! - Terrain is authored as 8 m x 8 m x 4 m *bands* (32 x 32 x 16 cells at
//!   25 cm). A band that is entirely below the surface shell is a placed
//!   instance of one of two shared, genuinely fully solid deep-base prototypes;
//!   a band that the surface, water, or a carved cavity passes through is a
//!   unique prototype holding real per-column shape. Nothing is padded with air
//!   and no cell is invented: every counted cell is a stored source cell of some
//!   prototype, and every instance places a prototype that exists.
//! - Every prototype stays at or below [`MAX_PROTOTYPE_CELLS`], the existing
//!   [`crate::MAX_MESH_BYTES`] preflight ceiling, so each one can actually be
//!   meshed at `Lod::Source`.
//! - Flora reuses the existing catalogue prototypes. A later catalogue worker
//!   adds more archetypes; this module places whatever
//!   [`crate::FLORA_SPECIES`] contains, so its species count is data driven and
//!   never claims types it did not place.
//!
//! Coordinates: world metres, `x` east, `y` up, `z` south. The landform sits on
//! a plateau datum: solid rock from `y = 0` up to the surface.

use crate::{
    bracket_fungus, flora_class, flora_prototype, material, material_policy, wetland_prototype,
    DetailError, DetailScene, DetailVolume, MaterialPolicy, Result, Scale, Transform, Yaw,
    FLORA_SPECIES, MAX_MESH_BYTES, SCALE_TILE_M,
};
use std::collections::BTreeMap;

/// Frozen showcase seed (owner requirement date 2026-09-08).
pub const SHOWCASE_SEED: u64 = 20_260_908;
/// Bump when generated content changes; snapshot hashes are version scoped.
pub const SHOWCASE_GENERATOR_VERSION: u32 = 3;

/// Playable square edge in metres.
pub const MAP_EDGE_M: f32 = 128.0;
/// Authoritative terrain cell size: 25 cm.
pub const TERRAIN_CELL_M: f32 = SCALE_TILE_M;
/// Terrain columns per map edge.
pub const MAP_EDGE_CELLS: i32 = 512;
/// Terrain tile footprint in cells (8 m).
pub const TILE_CELLS: i32 = 32;
/// Terrain band height in cells (4 m).
pub const BAND_CELLS: i32 = 16;
/// Tiles per map edge.
pub const TILES_PER_EDGE: i32 = MAP_EDGE_CELLS / TILE_CELLS;
/// Vertical cell ceiling; the landform is clamped well below this.
pub const MAX_TERRAIN_CELL_Y: i32 = 255;

/// Bytes one occupied cell can contribute to a derived mesh: 24 vertices plus
/// 36 indices. Mirrors the private bound used by [`crate::MAX_MESH_BYTES`].
const MESH_BYTES_PER_CELL: usize = 24 * std::mem::size_of::<matterweave_core::Vertex>() + 36 * 4;
/// Largest occupied-cell count whose `Lod::Source` mesh passes the existing
/// preflight. Terrain is split into bands so no prototype exceeds it.
pub const MAX_PROTOTYPE_CELLS: usize = MAX_MESH_BYTES / MESH_BYTES_PER_CELL;

/// Horizontal walk speed of the current native explorer character controller
/// (`apps/explorer/src/lib.rs`: input is clamped to unit length and scaled by 6).
pub const WALK_SPEED_M_S: f32 = 6.0;
/// Eye height above the walked surface, used only for [`Showcase::spawn_eye`].
pub const EYE_HEIGHT_M: f32 = 1.7;
/// Standing water surface of the basin, in metres above the datum.
pub const BASIN_WATER_LEVEL_M: f32 = 12.0;
/// Route sampling step in metres.
pub const ROUTE_STEP_M: f32 = 2.0;

/// Acceptance thresholds from `docs/SHOWCASE.md`. These are the owner targets,
/// asserted against generated authoritative data, never lowered here.
pub const SHOWCASE_EXPANDED_CELLS_MIN: usize = 20_000_000;
pub const SHOWCASE_FLORA_CELLS_MIN: usize = 2_000_000;
pub const SHOWCASE_PLANTS_MIN: usize = 4_000;

/// Every archetype the showcase places: the six accepted catalogue species plus
/// the four later original species (attached bracket fungus and the three
/// wetland prototypes). The source files of all ten are owned elsewhere and are
/// not modified here; this module only places them.
pub const SHOWCASE_SPECIES: [&str; 10] = [
    "parasol_mushroom",
    "funnel_mushroom",
    "clustered_mushroom",
    "fan_frond",
    "reed_cluster",
    "rosette_groundcover",
    "bracket_fungus",
    "twisted_shrub",
    "horsetail",
    "marsh_lily",
];

/// Half-width of the walked corridor kept free of plant *source* geometry, in
/// metres. A plant is rejected when its own horizontal source radius reaches
/// within this distance of any route point on either route, so an overhanging
/// cap or a woody branch cannot grow into the walked line even though its
/// placement origin is off the path.
pub const ROUTE_PLAYER_CLEARANCE_M: f32 = 0.9;

/// Builds any showcase archetype by id, across the three owning source modules.
pub fn showcase_prototype(id: &str) -> Result<DetailVolume> {
    match id {
        "bracket_fungus" => bracket_fungus(id),
        "twisted_shrub" | "horsetail" | "marsh_lily" => wetland_prototype(id),
        _ => flora_prototype(id),
    }
}

/// Largest horizontal distance in metres from a prototype's local origin to the
/// far side of any of its occupied cells. Taken over both horizontal axes, so
/// the value is invariant under the quarter-turn yaws used for placement.
fn source_radius_m(volume: &DetailVolume) -> f32 {
    let scale = volume.scale().metres();
    volume
        .iter_cells()
        .map(|(cell, _)| {
            // Farthest horizontal corner of each occupied voxel, not the box's
            // half-width: diagonal corners also need clearance from a walked path.
            let x = (cell[0] as f32).abs().max((cell[0] as f32 + 1.).abs()) * scale;
            let z = (cell[2] as f32).abs().max((cell[2] as f32 + 1.).abs()) * scale;
            x.hypot(z)
        })
        .fold(0., f32::max)
}

/// Horizontal source radius of one archetype in metres, measured from its
/// actual generated cells. Public so tests and tools use the same number the
/// scatter used.
pub fn species_source_radius_m(id: &str) -> Result<f32> {
    Ok(source_radius_m(&showcase_prototype(id)?))
}

// ---------------------------------------------------------------------------
// Deterministic noise
// ---------------------------------------------------------------------------

fn hash2(seed: u64, a: i32, b: i32) -> u64 {
    let mut v = seed
        ^ (a as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (b as i64 as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    v ^ (v >> 31)
}

/// Deterministic unit float in `[0, 1)` from 24 hashed bits.
fn unit(seed: u64, a: i32, b: i32) -> f32 {
    (hash2(seed, a, b) >> 40) as f32 / 16_777_216.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn value_noise(seed: u64, x: f32, z: f32) -> f32 {
    let xf = x.floor();
    let zf = z.floor();
    let (xi, zi) = (xf as i32, zf as i32);
    let (tx, tz) = (smooth(x - xf), smooth(z - zf));
    let n = |a: i32, b: i32| unit(seed, a, b) * 2.0 - 1.0;
    let a = n(xi, zi);
    let b = n(xi + 1, zi);
    let c = n(xi, zi + 1);
    let d = n(xi + 1, zi + 1);
    lerp(lerp(a, b, tx), lerp(c, d, tx), tz)
}

fn fbm(seed: u64, x: f32, z: f32, base_freq: f32, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut norm = 0.0;
    let mut amp = 1.0;
    let mut freq = base_freq;
    for octave in 0..octaves {
        sum += amp
            * value_noise(
                seed ^ (u64::from(octave) + 1).wrapping_mul(0x9e37_79b9),
                x * freq,
                z * freq,
            );
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

// ---------------------------------------------------------------------------
// Landform
// ---------------------------------------------------------------------------

/// Basin centre in metres.
pub const BASIN_CENTRE_M: [f32; 2] = [64.0, 84.0];
/// Basin radius used for the standing water mask.
const BASIN_RADIUS_M: f32 = 30.0;

/// Creek spine, north notch to basin. Authored landmark, not noise.
const CREEK_PATH: [[f32; 2]; 6] = [
    [42.0, 10.0],
    [46.0, 28.0],
    [52.0, 44.0],
    [56.0, 58.0],
    [60.0, 70.0],
    [63.0, 80.0],
];

/// Voxel cavities carved after the heightfield, producing real overhangs that a
/// heightfield alone cannot express: `(centre_m, radii_m)` ellipsoids.
const CAVITIES: [([f32; 3], [f32; 3]); 3] = [
    // West ridge rock shelter, mouth facing the basin.
    ([20.0, 20.0, 74.0], [6.5, 3.0, 5.0]),
    // East terrace undercut below the elevated route.
    ([100.0, 17.0, 60.0], [5.0, 2.6, 4.5]),
    // West slope undercut above the grove.
    ([26.0, 19.0, 60.0], [4.0, 2.0, 3.5]),
];

fn dist2(ax: f32, az: f32, bx: f32, bz: f32) -> f32 {
    let (dx, dz) = (ax - bx, az - bz);
    (dx * dx + dz * dz).sqrt()
}

/// Distance to the creek spine and normalised progress along it.
fn creek_distance(x: f32, z: f32) -> (f32, f32) {
    let mut lengths = [0.0f32; CREEK_PATH.len()];
    let mut total = 0.0;
    for i in 1..CREEK_PATH.len() {
        lengths[i - 1] = dist2(
            CREEK_PATH[i - 1][0],
            CREEK_PATH[i - 1][1],
            CREEK_PATH[i][0],
            CREEK_PATH[i][1],
        );
        total += lengths[i - 1];
    }
    let mut best = f32::INFINITY;
    let mut best_t = 0.0;
    let mut travelled = 0.0;
    for i in 1..CREEK_PATH.len() {
        let a = CREEK_PATH[i - 1];
        let b = CREEK_PATH[i];
        let (vx, vz) = (b[0] - a[0], b[1] - a[1]);
        let len2 = vx * vx + vz * vz;
        let t = if len2 > 0.0 {
            (((x - a[0]) * vx + (z - a[1]) * vz) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = dist2(x, z, a[0] + vx * t, a[1] + vz * t);
        if d < best {
            best = d;
            best_t = (travelled + lengths[i - 1] * t) / total;
        }
        travelled += lengths[i - 1];
    }
    (best, best_t)
}

/// Water surface height in metres at a column, or `None` where the column is dry.
fn water_level_m(x: f32, z: f32) -> Option<f32> {
    let basin = dist2(x, z, BASIN_CENTRE_M[0], BASIN_CENTRE_M[1]) < BASIN_RADIUS_M;
    let (creek_d, creek_t) = creek_distance(x, z);
    let creek = creek_d < 3.0;
    match (basin, creek) {
        (false, false) => None,
        (true, false) => Some(BASIN_WATER_LEVEL_M),
        _ => {
            let creek_surface = lerp(19.6, BASIN_WATER_LEVEL_M, smooth(creek_t));
            Some(if basin {
                creek_surface.max(BASIN_WATER_LEVEL_M)
            } else {
                creek_surface
            })
        }
    }
}

fn flatten(h: f32, d: f32, radius: f32, target: f32, strength: f32) -> f32 {
    let w = smooth((1.0 - (d / radius).min(1.0)).max(0.0)) * strength;
    lerp(h, target, w)
}

/// Natural landform height before the authored trail corridors are graded in.
fn base_height_m(seed: u64, x: f32, z: f32) -> f32 {
    let border = x.min(MAP_EDGE_M - x).min(z).min(MAP_EDGE_M - z);
    // Ash ridge ring with a varied silhouette and a southern saddle exit.
    let ridge_shape = ((30.0 - border) / 30.0).clamp(0.0, 1.0);
    let ridge_var = 0.72 + 0.55 * (fbm(seed ^ 0xa1, x, z, 0.03, 2) * 0.5 + 0.5);
    let gate = (1.0 - ((x - 64.0).abs() / 11.0).min(1.0)) * ((z - 104.0) / 20.0).clamp(0.0, 1.0);
    let ridge = 32.0 * ridge_shape.powf(1.8) * ridge_var * (1.0 - 0.78 * gate);

    // Drainage bowl toward the basin.
    let dc = dist2(x, z, BASIN_CENTRE_M[0], BASIN_CENTRE_M[1]);
    let bowl = (9.2 + 0.0060 * dc * dc).min(22.0);
    let mut h = bowl + ridge;

    // Eastern terraces: quantised steps blended in with distance from the basin.
    let terrace_t = ((x - 84.0) / 16.0).clamp(0.0, 1.0);
    if terrace_t > 0.0 {
        let step = 3.0;
        let stepped = (h / step).floor() * step + 0.4;
        h = lerp(h, stepped, terrace_t * 0.8);
    }

    // Local relief before erosion, so gullies cut through it.
    h += 1.9 * fbm(seed ^ 0xb2, x, z, 0.09, 4) + 0.55 * fbm(seed ^ 0xb3, x, z, 0.33, 2);

    // Radial erosion gullies feeding the basin.
    for k in 0..6i32 {
        let angle = (unit(seed ^ 0xc4, k, 0) * std::f32::consts::TAU) + k as f32 * 0.31;
        let (sx, sz) = (angle.cos(), angle.sin());
        let rel_x = x - BASIN_CENTRE_M[0];
        let rel_z = z - BASIN_CENTRE_M[1];
        let along = rel_x * sx + rel_z * sz;
        if along < 18.0 {
            continue;
        }
        let across =
            (rel_x * -sz + rel_z * sx).abs() + 1.6 * fbm(seed ^ (0xd5 + k as u64), x, z, 0.06, 2);
        let depth = 2.4
            * (-(across / 2.0) * (across / 2.0)).exp()
            * ((along - 18.0) / 12.0).clamp(0.0, 1.0);
        h -= depth;
    }

    // Authored landmark shelves.
    h = flatten(h, dist2(x, z, 42.0, 62.0), 10.0, 15.4, 0.85); // sheltered fungal grove
    h = flatten(h, dist2(x, z, 92.0, 58.0), 6.0, 18.2, 0.9); // destruction clearing
    h = flatten(h, dist2(x, z, 106.0, 30.0), 7.0, 33.5, 0.85); // high viewpoint
    h = flatten(h, dist2(x, z, 64.0, 112.0), 8.0, 14.0, 0.7); // south saddle approach

    // Creek channel: guarantees a wet bed under the creek water surface.
    let (creek_d, creek_t) = creek_distance(x, z);
    if creek_d < 7.0 {
        let surface = lerp(19.6, BASIN_WATER_LEVEL_M, smooth(creek_t));
        let bank = smooth((1.0 - (creek_d / 7.0)).max(0.0));
        let bed = surface - 0.55 + 0.9 * (creek_d / 7.0);
        h = lerp(h, h.min(bed), bank);
    }

    h.clamp(0.5, 46.0)
}

// ---------------------------------------------------------------------------
// Trail corridors
// ---------------------------------------------------------------------------

/// Full-strength half width of a graded trail, in metres. Wide enough for the
/// 0.60 m character with shoulder room, narrow enough to leave the terraces,
/// ridges and gullies that give the map its silhouette intact.
const TRAIL_HALF_WIDTH_M: f32 = 1.6;
/// Distance over which a trail blends back into natural ground.
const TRAIL_VERGE_M: f32 = 3.2;
/// Spacing of the trail elevation profile along the corridor, in metres.
const TRAIL_PROFILE_STEP_M: f32 = 0.5;
/// Half length of the moving average that grades the profile, in metres. A
/// 2.4 m terrace riser becomes a ramp of roughly 0.25 rise per metre of run.
const TRAIL_GRADE_M: f32 = 7.0;
/// Rounds of crossing agreement between corridors.
const TRAIL_RELAX_ROUNDS: usize = 6;
/// Samples within this index distance along the *same* corridor are its own
/// neighbours, not a crossing, and are excluded from the agreement pass.
const TRAIL_SELF_SKIP: usize = 24;
/// Steepest walking grade a corridor may keep, in metres of rise per metre of
/// run. Smoothing alone cannot fix a sustained natural slope, and the terrain
/// classifier refuses anything above 0.85 as a cliff, so the profile is also
/// gradient limited to 0.30, about 17 degrees before voxel quantization.
const TRAIL_MAX_GRADE: f32 = 0.30;

/// Arc-length elevation profile of one authored corridor.
///
/// Grading a walking corridor into generated terrain is the standard road/path
/// pass (the same idea as the landmark shelves this map already grades with
/// [`flatten`]): sample the natural ground along the spine, smooth that profile
/// until its gradient is walkable, then blend it back in over a narrow verge.
/// The alternative considered and rejected was leaving the terrain untouched
/// and routing around every riser: the eastern terraces quantise to 3 m steps,
/// so no continuous ground route to the viewpoint exists at all.
struct TrailProfile {
    /// Graded height at every [`TRAIL_PROFILE_STEP_M`] of arc length.
    heights: Vec<f32>,
}

impl TrailProfile {
    /// Samples the natural ground along one spine at [`TRAIL_PROFILE_STEP_M`].
    fn sample(seed: u64, spine: &[[f32; 2]], close: bool) -> (Self, Vec<[f32; 2]>) {
        let mut points: Vec<[f32; 2]> = spine.to_vec();
        if close {
            points.push(spine[0]);
        }
        let arc = arc_lengths(&points);
        let total = *arc.last().expect("seeded");
        let samples = (total / TRAIL_PROFILE_STEP_M).ceil() as usize + 1;
        let positions: Vec<[f32; 2]> = (0..samples)
            .map(|i| point_on_polyline(&points, &arc, (i as f32 * TRAIL_PROFILE_STEP_M).min(total)))
            .collect();
        let heights = positions
            .iter()
            .map(|p| base_height_m(seed, p[0], p[1]))
            .collect();
        (Self { heights }, positions)
    }

    /// Moving average along the corridor, then a gradient limit. Smoothing
    /// shortens a riser; only the limit bounds a sustained natural slope,
    /// which is what makes the ridge and terrace legs walkable at all.
    fn grade_profile(&mut self) {
        let samples = self.heights.len();
        let window = (TRAIL_GRADE_M / TRAIL_PROFILE_STEP_M) as usize;
        self.heights = (0..samples)
            .map(|i| {
                let lo = i.saturating_sub(window);
                let hi = (i + window).min(samples - 1);
                let span = &self.heights[lo..=hi];
                span.iter().sum::<f32>() / span.len() as f32
            })
            .collect();
        self.limit_gradient();
    }

    fn limit_gradient(&mut self) {
        let samples = self.heights.len();
        let limit = TRAIL_MAX_GRADE * TRAIL_PROFILE_STEP_M;
        for _ in 0..4 {
            for i in 1..samples {
                let previous = self.heights[i - 1];
                self.heights[i] = self.heights[i].clamp(previous - limit, previous + limit);
            }
            for i in (0..samples - 1).rev() {
                let next = self.heights[i + 1];
                self.heights[i] = self.heights[i].clamp(next - limit, next + limit);
            }
        }
    }

    fn height_at(&self, s: f32) -> f32 {
        let t = (s / TRAIL_PROFILE_STEP_M).max(0.0);
        let i = (t.floor() as usize).min(self.heights.len() - 1);
        let j = (i + 1).min(self.heights.len() - 1);
        lerp(self.heights[i], self.heights[j], t - t.floor())
    }
}

fn arc_lengths(points: &[[f32; 2]]) -> Vec<f32> {
    let mut arc = vec![0.0f32];
    for pair in points.windows(2) {
        let last = *arc.last().expect("seeded");
        arc.push(last + dist2(pair[0][0], pair[0][1], pair[1][0], pair[1][1]));
    }
    arc
}

fn point_on_polyline(points: &[[f32; 2]], arc: &[f32], s: f32) -> [f32; 2] {
    for i in 1..points.len() {
        if s <= arc[i] || i + 1 == points.len() {
            let span = (arc[i] - arc[i - 1]).max(1e-6);
            let t = ((s - arc[i - 1]) / span).clamp(0.0, 1.0);
            return [
                lerp(points[i - 1][0], points[i][0], t),
                lerp(points[i - 1][1], points[i][1], t),
            ];
        }
    }
    points[0]
}

/// Distance to a polyline and the arc length of the closest point on it.
fn polyline_distance(points: &[[f32; 2]], arc: &[f32], x: f32, z: f32) -> (f32, f32) {
    let mut best = f32::INFINITY;
    let mut best_s = 0.0;
    for i in 1..points.len() {
        let a = points[i - 1];
        let b = points[i];
        let (vx, vz) = (b[0] - a[0], b[1] - a[1]);
        let len2 = vx * vx + vz * vz;
        let t = if len2 > 0.0 {
            (((x - a[0]) * vx + (z - a[1]) * vz) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = dist2(x, z, a[0] + vx * t, a[1] + vz * t);
        if d < best {
            best = d;
            best_s = arc[i - 1] + (arc[i] - arc[i - 1]) * t;
        }
    }
    (best, best_s)
}

/// Authored corridors and arc tables, built once from the canonical seed.
/// Every one of the map's 262,144 columns queries this immutable profile.
struct Trails {
    spines: [(Vec<[f32; 2]>, Vec<f32>, TrailProfile); 2],
}

impl Trails {
    fn build(seed: u64) -> Self {
        let specs: [(&[[f32; 2]], bool); 2] = [(ROUTE_SPINE, true), (&ELEVATED_SPINE, false)];
        let mut profiles = Vec::new();
        let mut positions = Vec::new();
        let mut polylines = Vec::new();
        for (spine, close) in specs {
            let (profile, samples) = TrailProfile::sample(seed, spine, close);
            let mut points: Vec<[f32; 2]> = spine.to_vec();
            if close {
                points.push(spine[0]);
            }
            polylines.push((points.clone(), arc_lengths(&points)));
            profiles.push(profile);
            positions.push(samples);
        }
        for profile in &mut profiles {
            profile.grade_profile();
        }
        // The ground loop crosses itself, and the elevated spur leaves and
        // rejoins it. Grading each corridor independently left a step of
        // several metres where two corridors meet, which is a wall exactly
        // where the route needs to be continuous. Relax the crossings until
        // every corridor agrees where they overlap, re-limiting the gradient
        // after each round so the agreement stays walkable.
        for _ in 0..TRAIL_RELAX_ROUNDS {
            let snapshot: Vec<Vec<f32>> = profiles.iter().map(|p| p.heights.clone()).collect();
            for (pi, profile) in profiles.iter_mut().enumerate() {
                for (si, height) in profile.heights.iter_mut().enumerate() {
                    let here = positions[pi][si];
                    let mut sum = *height;
                    let mut count = 1.0f32;
                    for (qi, other) in positions.iter().enumerate() {
                        for (ti, there) in other.iter().enumerate() {
                            if pi == qi && si.abs_diff(ti) < TRAIL_SELF_SKIP {
                                continue;
                            }
                            if dist2(here[0], here[1], there[0], there[1]) < TRAIL_VERGE_M {
                                sum += snapshot[qi][ti];
                                count += 1.0;
                            }
                        }
                    }
                    *height = sum / count;
                }
            }
            for profile in &mut profiles {
                profile.limit_gradient();
            }
        }
        let mut spines = profiles.into_iter().zip(polylines);
        let mut take = || {
            let (profile, (points, arc)) = spines.next().expect("two corridors");
            (points, arc, profile)
        };
        Self {
            spines: [take(), take()],
        }
    }

    /// Graded height and blend weight at a column, or `None` off every trail.
    ///
    /// Corridors are combined by weight rather than by nearest spine. Where the
    /// loop runs beside the elevated spur, picking the nearer corridor left a
    /// seam between two slightly different graded heights, and that seam was
    /// steeper than the ground the character can walk.
    fn grade(&self, x: f32, z: f32) -> Option<(f32, f32)> {
        let mut sum = 0.0f32;
        let mut total = 0.0f32;
        let mut strongest = 0.0f32;
        for (points, arc, profile) in &self.spines {
            let (d, s) = polyline_distance(points, arc, x, z);
            if d >= TRAIL_VERGE_M {
                continue;
            }
            let verge = smooth(
                ((TRAIL_VERGE_M - d) / (TRAIL_VERGE_M - TRAIL_HALF_WIDTH_M)).clamp(0.0, 1.0),
            );
            // Weight by proximity as well as by verge strength, so the join
            // between two corridors is a smooth blend and never a step.
            let weight = verge + 1e-3;
            sum += profile.height_at(s) * weight;
            total += weight;
            strongest = strongest.max(verge);
        }
        (total > 0.0).then(|| (sum / total, strongest))
    }
}

/// The paths are authored landmarks, like the basin and clearing. Their graded
/// elevation profiles stay fixed across world seeds; surrounding terrain and
/// flora still vary. One immutable profile avoids per-column locks and prevents
/// nearby random terrain variations from changing the designed trail grade.
fn trails() -> &'static Trails {
    static TRAILS: std::sync::OnceLock<Trails> = std::sync::OnceLock::new();
    TRAILS.get_or_init(|| Trails::build(SHOWCASE_SEED))
}

/// Authoritative terrain height in metres. Coherent basin, creek, drainage,
/// ash ridges, gullies, terraces and authored landmark shelves, with the two
/// authored walking corridors graded into that landform.
pub fn terrain_height_m(seed: u64, x: f32, z: f32) -> f32 {
    let h = base_height_m(seed, x, z);
    match trails().grade(x, z) {
        Some((target, weight)) => lerp(h, target, weight).clamp(0.5, 46.0),
        None => h,
    }
}

// ---------------------------------------------------------------------------
// Terrain grid and public surface query
// ---------------------------------------------------------------------------

/// One authoritative surface sample, in metres, for scatter, routing and tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfacePoint {
    /// Top solid cell index (25 cm units) of the column.
    pub top_cell: i32,
    /// Walkable surface height: the top of the top solid cell, in metres.
    pub height_m: f32,
    /// Material of the top solid cell.
    pub material: u8,
    /// Local gradient magnitude (metres of rise per metre of run).
    pub slope: f32,
    /// Standing water depth above the surface in metres; 0 when dry.
    pub water_depth_m: f32,
    /// True when this column is a real overhang: carved-away air sits *below*
    /// the reported surface, so the reported top is a roof over a void. The
    /// reported surface itself is always the highest uncarved solid cell, so a
    /// column whose original top was carved away reports the cavity floor (an
    /// open mouth) and is not marked here.
    pub overhung: bool,
}

impl SurfacePoint {
    pub fn is_dry_land(&self) -> bool {
        self.water_depth_m <= 0.0
    }
}

/// The authoritative terrain sampling grid: one column record per 25 cm column.
/// Voxel bands and every metre-space query are derived from this single grid, so
/// a query can never disagree with the generated voxels.
pub struct Terrain {
    seed: u64,
    /// Highest solid cell of the *uncarved* heightfield column. Depth-based
    /// material layering is measured from this, so carving a shelter does not
    /// silently turn deep rock into surface turf.
    raw_top: Vec<i32>,
    /// Highest cell that is still solid *after* cavity carving. This is what
    /// every metre-space query reports, so a query can never name a cell the
    /// generator carved away.
    top: Vec<i32>,
    water_top: Vec<i32>,
    top_material: Vec<u8>,
    /// Column has solid material above a carved void: a true overhang.
    overhung: Vec<bool>,
}

/// Cheap rejection: does this column's centre lie within any cavity's
/// horizontal bounding box? Only such columns can be affected by carving.
fn column_touches_cavity(x: f32, z: f32) -> bool {
    CAVITIES.iter().any(|(centre, radii)| {
        (x - centre[0]).abs() <= radii[0] && (z - centre[2]).abs() <= radii[2]
    })
}

fn column_index(x: i32, z: i32) -> Option<usize> {
    ((0..MAP_EDGE_CELLS).contains(&x) && (0..MAP_EDGE_CELLS).contains(&z))
        .then_some((x * MAP_EDGE_CELLS + z) as usize)
}

impl Terrain {
    /// Builds the grid once. Cost is bounded: `MAP_EDGE_CELLS^2` columns.
    pub fn generate(seed: u64) -> Result<Self> {
        let cells = (MAP_EDGE_CELLS * MAP_EDGE_CELLS) as usize;
        let mut raw_top = vec![0i32; cells];
        let mut top = vec![0i32; cells];
        let mut overhung = vec![false; cells];
        let mut water_top = vec![-1i32; cells];
        let top_material = vec![material::AIR; cells];
        for xi in 0..MAP_EDGE_CELLS {
            let x = (xi as f32 + 0.5) * TERRAIN_CELL_M;
            for zi in 0..MAP_EDGE_CELLS {
                let z = (zi as f32 + 0.5) * TERRAIN_CELL_M;
                let h = terrain_height_m(seed, x, z);
                let t = (h / TERRAIN_CELL_M).floor() as i32 - 1;
                if !(0..=MAX_TERRAIN_CELL_Y).contains(&t) {
                    return Err(DetailError::BudgetExceeded(
                        "terrain height out of cell range",
                    ));
                }
                let idx = (xi * MAP_EDGE_CELLS + zi) as usize;
                raw_top[idx] = t;
                // Cavity carving is applied here, not only in the voxel pass, so
                // the reported surface is the highest cell that survives it.
                let touches = column_touches_cavity(x, z);
                let mut carved_top = t;
                while touches
                    && carved_top >= 0
                    && carved(x, (carved_top as f32 + 0.5) * TERRAIN_CELL_M, z)
                {
                    carved_top -= 1;
                }
                if carved_top < 0 {
                    return Err(DetailError::BudgetExceeded("cavity carved a whole column"));
                }
                top[idx] = carved_top;
                let surface_m = (t + 1) as f32 * TERRAIN_CELL_M;
                if let Some(level) = water_level_m(x, z) {
                    if level > surface_m {
                        let wt = (level / TERRAIN_CELL_M).floor() as i32 - 1;
                        if wt > t {
                            water_top[idx] = wt.min(MAX_TERRAIN_CELL_Y);
                        }
                    }
                }
                // A true overhang: a carved cell that stays air (it is above the
                // standing water surface, so it does not flood) below the solid
                // cell this column reports as its surface.
                if touches {
                    overhung[idx] = (0..carved_top).any(|y| {
                        y > water_top[idx] && carved(x, (y as f32 + 0.5) * TERRAIN_CELL_M, z)
                    });
                }
            }
        }
        let mut terrain = Self {
            seed,
            raw_top,
            top,
            water_top,
            top_material,
            overhung,
        };
        // Surface materials need the finished height grid, because they depend
        // on the local slope of neighbouring columns.
        let mut classified = vec![material::AIR; cells];
        for xi in 0..MAP_EDGE_CELLS {
            for zi in 0..MAP_EDGE_CELLS {
                classified[(xi * MAP_EDGE_CELLS + zi) as usize] = terrain.classify(xi, zi);
            }
        }
        terrain.top_material = classified;
        Ok(terrain)
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    fn top_cell_at(&self, x: i32, z: i32) -> i32 {
        match column_index(x, z) {
            Some(idx) => self.top[idx],
            // Outside the map the nearest in-range column is used only for slope.
            None => {
                let cx = x.clamp(0, MAP_EDGE_CELLS - 1);
                let cz = z.clamp(0, MAP_EDGE_CELLS - 1);
                self.top[(cx * MAP_EDGE_CELLS + cz) as usize]
            }
        }
    }

    fn slope_cells(&self, x: i32, z: i32) -> f32 {
        let dx = (self.top_cell_at(x + 1, z) - self.top_cell_at(x - 1, z)) as f32 * 0.5;
        let dz = (self.top_cell_at(x, z + 1) - self.top_cell_at(x, z - 1)) as f32 * 0.5;
        // Cell units cancel: rise in cells over run in cells is dimensionless.
        (dx * dx + dz * dz).sqrt()
    }

    fn classify(&self, x: i32, z: i32) -> u8 {
        let idx = (x * MAP_EDGE_CELLS + z) as usize;
        let top = self.top[idx];
        if self.water_top[idx] > top {
            return material::BANK_STONE;
        }
        let height = (top + 1) as f32 * TERRAIN_CELL_M;
        let slope = self.slope_cells(x, z);
        if slope > 0.9 || height > 27.0 {
            return material::BANK_STONE;
        }
        let wx = (x as f32 + 0.5) * TERRAIN_CELL_M;
        let wz = (z as f32 + 0.5) * TERRAIN_CELL_M;
        match water_level_m(wx, wz) {
            Some(level) if height < level + 0.9 => material::DETAIL_SOIL,
            _ => material::MOSS_TURF,
        }
    }

    /// Material of a cell inside the terrain column, before cavity carving.
    /// Depth layering follows the uncarved column; the classified surface
    /// material is placed on the highest cell that survives carving, which is
    /// exactly the cell [`Terrain::surface_at_metres`] reports.
    fn column_material(&self, idx: usize, y: i32) -> u8 {
        let top = self.raw_top[idx];
        if y > top {
            return if y <= self.water_top[idx] {
                material::WATER
            } else {
                material::AIR
            };
        }
        if y == self.top[idx] {
            return self.top_material[idx];
        }
        if y > top - 6 {
            material::DETAIL_SOIL
        } else {
            material::BANK_STONE
        }
    }

    /// Public reusable surface query in world metres. `None` outside the map.
    pub fn surface_at_metres(&self, x_m: f32, z_m: f32) -> Option<SurfacePoint> {
        if !x_m.is_finite() || !z_m.is_finite() {
            return None;
        }
        let xi = (x_m / TERRAIN_CELL_M).floor() as i32;
        let zi = (z_m / TERRAIN_CELL_M).floor() as i32;
        let idx = column_index(xi, zi)?;
        let top = self.top[idx];
        let height_m = (top + 1) as f32 * TERRAIN_CELL_M;
        let water_depth_m = if self.water_top[idx] > top {
            (self.water_top[idx] + 1) as f32 * TERRAIN_CELL_M - height_m
        } else {
            0.0
        };
        Some(SurfacePoint {
            top_cell: top,
            height_m,
            material: self.top_material[idx],
            slope: self.slope_cells(xi, zi),
            water_depth_m,
            overhung: self.overhung[idx],
        })
    }

    /// Walkable surface height in metres, or `None` outside the map.
    pub fn height_at_metres(&self, x_m: f32, z_m: f32) -> Option<f32> {
        self.surface_at_metres(x_m, z_m).map(|p| p.height_m)
    }

    /// Water surface height in metres where the column is flooded.
    pub fn water_surface_at_metres(&self, x_m: f32, z_m: f32) -> Option<f32> {
        let p = self.surface_at_metres(x_m, z_m)?;
        (p.water_depth_m > 0.0).then_some(p.height_m + p.water_depth_m)
    }
}

/// Centre of the terrain cell containing a metre coordinate, so cavity tests
/// agree with the cells that were actually carved.
pub fn cell_centre_m(value: f32) -> f32 {
    ((value / TERRAIN_CELL_M).floor() + 0.5) * TERRAIN_CELL_M
}

/// True when a cell centre lies inside a carved cavity.
pub fn carved(x_m: f32, y_m: f32, z_m: f32) -> bool {
    CAVITIES.iter().any(|(centre, radii)| {
        let dx = (x_m - centre[0]) / radii[0];
        let dy = (y_m - centre[1]) / radii[1];
        let dz = (z_m - centre[2]) / radii[2];
        dx * dx + dy * dy + dz * dz <= 1.0
    })
}

/// Lowest cavity cell index touching a tile footprint, if any.
fn tile_cavity_floor_cell(tile_x: i32, tile_z: i32) -> Option<i32> {
    let x0 = (tile_x * TILE_CELLS) as f32 * TERRAIN_CELL_M;
    let z0 = (tile_z * TILE_CELLS) as f32 * TERRAIN_CELL_M;
    let x1 = x0 + TILE_CELLS as f32 * TERRAIN_CELL_M;
    let z1 = z0 + TILE_CELLS as f32 * TERRAIN_CELL_M;
    let mut floor: Option<i32> = None;
    for (centre, radii) in CAVITIES {
        if centre[0] + radii[0] < x0
            || centre[0] - radii[0] > x1
            || centre[2] + radii[2] < z0
            || centre[2] - radii[2] > z1
        {
            continue;
        }
        let cell = ((centre[1] - radii[1]) / TERRAIN_CELL_M).floor() as i32 - 1;
        floor = Some(floor.map_or(cell, |f: i32| f.min(cell)));
    }
    floor
}

// ---------------------------------------------------------------------------
// Landmarks, route and manifest
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Landmark {
    pub name: &'static str,
    pub kind: &'static str,
    pub position_m: [f32; 3],
}

/// Per-class content accounting. Prototype cells and instance-expanded cells are
/// reported separately and are never conflated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassCounts {
    pub prototypes: usize,
    pub instances: usize,
    pub unique_stored_cells: usize,
    pub expanded_cells: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentManifest {
    pub generator_version: u32,
    pub seed: u64,
    pub classes: BTreeMap<String, ClassCounts>,
    /// Instances per placed flora prototype id.
    pub species_instances: BTreeMap<String, usize>,
    pub terrain_shell_prototypes: usize,
    pub terrain_base_prototypes: usize,
    pub terrain_base_instances: usize,
    pub flora_instances: usize,
    pub flora_expanded_cells: usize,
    pub terrain_expanded_cells: usize,
    pub water_expanded_cells: usize,
    pub overhang_columns: usize,
    pub route_length_m: f32,
    pub route_walk_seconds: f32,
    pub largest_prototype_cells: usize,
}

/// The generated showcase: authoritative scene plus the derived navigation and
/// accounting data the renderer, tests and manifests need.
pub struct Showcase {
    pub scene: DetailScene,
    pub terrain: Terrain,
    pub spawn_eye: [f32; 3],
    /// Ground-level loop route in world metres, densified at [`ROUTE_STEP_M`].
    pub route: Vec<[f32; 3]>,
    /// Elevated alternate route over the eastern terraces to the viewpoint.
    pub elevated_route: Vec<[f32; 3]>,
    pub landmarks: Vec<Landmark>,
    pub manifest: ContentManifest,
}

impl Showcase {
    /// Short waterside itinerary beginning at the grove entrance.
    pub fn waterside_route(&self) -> &[[f32; 3]] {
        // Follow the initial shoreward section to the west basin bend. It is
        // an exact prefix, so the full continuous traversal also tests this
        // itinerary without inventing a shortcut across uncrossable terrain.
        let end = self.route.iter().position(|p| p[2] >= 90.0 && p[0] <= 48.0);
        end.map_or(&[], |end| &self.route[..=end])
    }

    /// Loop length in metres including climb.
    pub fn route_length_m(&self) -> f32 {
        polyline_length(&self.route)
    }
    /// Loop traversal seconds at the current explorer walk speed. Host estimate
    /// only; phone traversal is a separate measurement.
    pub fn route_walk_seconds(&self) -> f32 {
        self.route_length_m() / WALK_SPEED_M_S
    }
}

fn polyline_length(points: &[[f32; 3]]) -> f32 {
    points
        .windows(2)
        .map(|w| {
            let d: [f32; 3] = std::array::from_fn(|a| w[1][a] - w[0][a]);
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        })
        .sum()
}

/// Coarse ground loop spine: waterside, grove serpentine, creek ascent, west
/// ridge traverse, eastern terrace climb, viewpoint spur, clearing, saddle.
const ROUTE_SPINE: &[[f32; 2]] = &[
    [46.0, 66.0],
    [50.0, 74.0],
    [46.0, 82.0],
    [42.0, 90.0],
    [46.0, 98.0],
    [54.0, 104.0],
    [64.0, 110.0],
    [74.0, 106.0],
    [82.0, 100.0],
    [88.0, 92.0],
    [92.0, 84.0],
    [96.0, 76.0],
    [92.0, 68.0],
    [86.0, 62.0],
    [92.0, 58.0],
    [98.0, 54.0],
    [102.0, 46.0],
    [106.0, 38.0],
    [106.0, 30.0],
    [110.0, 24.0],
    [104.0, 20.0],
    [96.0, 22.0],
    [90.0, 28.0],
    [84.0, 34.0],
    [78.0, 30.0],
    [72.0, 24.0],
    [64.0, 20.0],
    [56.0, 16.0],
    [48.0, 14.0],
    [42.0, 18.0],
    [40.0, 26.0],
    [34.0, 30.0],
    [26.0, 34.0],
    [22.0, 42.0],
    [18.0, 50.0],
    [16.0, 58.0],
    [20.0, 66.0],
    [22.0, 74.0],
    [28.0, 78.0],
    [34.0, 74.0],
    [36.0, 66.0],
    [32.0, 58.0],
    [34.0, 50.0],
    [40.0, 46.0],
    [44.0, 54.0],
    [46.0, 60.0],
    // Second circuit: creek ascent, western gullies, northern shore and the
    // eastern terrace return leg. The loop is long on purpose: the explorer
    // character walks at 6 m/s, so a 3-5 minute loop needs over a kilometre.
    [50.0, 52.0],
    [54.0, 46.0],
    [52.0, 38.0],
    [48.0, 32.0],
    [44.0, 26.0],
    [38.0, 22.0],
    [32.0, 26.0],
    [28.0, 32.0],
    [24.0, 40.0],
    [20.0, 48.0],
    [14.0, 56.0],
    [12.0, 64.0],
    [16.0, 72.0],
    [14.0, 80.0],
    [18.0, 88.0],
    [24.0, 94.0],
    [30.0, 100.0],
    [36.0, 106.0],
    [44.0, 110.0],
    [52.0, 114.0],
    [60.0, 118.0],
    [68.0, 116.0],
    [76.0, 112.0],
    [84.0, 106.0],
    [90.0, 98.0],
    [96.0, 90.0],
    [100.0, 82.0],
    [104.0, 74.0],
    [100.0, 66.0],
    [96.0, 60.0],
    [90.0, 54.0],
    [84.0, 50.0],
    [78.0, 46.0],
    [72.0, 42.0],
    [66.0, 46.0],
    [60.0, 50.0],
    [54.0, 56.0],
    [50.0, 62.0],
];

const ELEVATED_SPINE: [[f32; 2]; 10] = [
    [86.0, 62.0],
    [92.0, 56.0],
    [97.0, 50.0],
    [100.0, 44.0],
    [103.0, 38.0],
    [106.0, 32.0],
    [108.0, 26.0],
    [103.0, 22.0],
    [97.0, 24.0],
    [92.0, 30.0],
];

/// Largest rise a walking character may take between adjacent 25 cm terrain
/// cells: one source cell, 0.25 m, well inside the character controller's
/// autostep. A rejected 0.55 m controller experiment trapped the capsule on
/// narrow treads; the accepted controller and routes use the smaller bounds.
const NAV_STEP_CELLS: i32 = 1;
/// Local quantized slope allowance. A gentle diagonal ramp can report sqrt(0.5)
/// from quarter-metre central differences. Capsule shoulder clearance and edge
/// heights below impose the actual walking constraints; source tests are followed
/// by the complete Rapier traversal gate.
const NAV_MAX_SLOPE: f32 = 0.85;
/// Deepest standing water a route may wade, in metres. The creek and the basin
/// shallows are genuine crossings the app already models at wading speed;
/// anything deeper is water the route must go around.
const NAV_WADE_DEPTH_M: f32 = 0.5;
/// Extra search cost of a waded cell. Bias routes toward dry ground; this is a
/// preference cost, not a prediction of actual wading duration.
const NAV_WADE_PENALTY: i64 = 6_000;
/// Chebyshev radius, in cells, inspected for taller terrain beside a route
/// cell. The capsule radius is 0.30 m, so a route hugging a wall scrapes it;
/// such cells are excluded. The graded corridor keeps a broad enough tread;
/// ungraded narrow terraces are not reliable capsule walking surfaces.
const NAV_CLEARANCE_CELLS: i32 = 2;
/// Route search bound per spine segment, in expanded cells.
const NAV_MAX_EXPANSIONS: usize = 400_000;
/// Ring search bound, in cells, when snapping a spine anchor onto the grid.
const NAV_SNAP_CELLS: i32 = 56;

/// Derived walking connectivity of the terrain, at the terrain's own 25 cm
/// column resolution. This is *derived* data: the terrain grid stays
/// authoritative and nothing here changes a single voxel.
///
/// A route point on its own proves nothing about walking. Independently
/// snapping route samples to nearby standable columns produced waypoint pairs
/// separated by a 0.5-1.9 m wall, which the actual character controller cannot
/// cross. Connectivity therefore has to be a property of the *edges*, which is
/// what this grid stores.
struct NavGrid {
    /// Column may carry a route point: dry, gentle and with real footing.
    walkable: Vec<bool>,
    /// Column carries standing water shallow enough to wade.
    waded: Vec<bool>,
    /// Column stands within shoulder reach of terrain the character cannot
    /// step onto, so a route through it scrapes a wall.
    /// Top solid cell per column, copied for cheap edge tests.
    top: Vec<i32>,
}

fn nav_index(x: i32, z: i32) -> Option<usize> {
    column_index(x, z)
}

impl NavGrid {
    fn build(terrain: &Terrain) -> Self {
        let cells = (MAP_EDGE_CELLS * MAP_EDGE_CELLS) as usize;
        let top = terrain.top.clone();
        // Standable columns with physical footing and bounded water depth.
        // Solid cave roofs are valid support: their authoritative top cells
        // exist. Quantized slope alone is insufficient; shoulder room follows.
        let mut walkable = vec![false; cells];
        let mut waded = vec![false; cells];
        for x in 0..MAP_EDGE_CELLS {
            for z in 0..MAP_EDGE_CELLS {
                let idx = (x * MAP_EDGE_CELLS + z) as usize;
                let depth =
                    ((terrain.water_top[idx] - terrain.top[idx]) as f32 * TERRAIN_CELL_M).max(0.0);
                walkable[idx] =
                    depth <= NAV_WADE_DEPTH_M && terrain.slope_cells(x, z) <= NAV_MAX_SLOPE;
                waded[idx] = walkable[idx] && depth > 0.0;
            }
        }
        // A cell is pinched when a nearby riser exceeds one source step within
        // the capsule's shoulder footprint. Do not use these narrow treads.
        let mut pinched = vec![false; cells];
        for x in 0..MAP_EDGE_CELLS {
            for z in 0..MAP_EDGE_CELLS {
                let idx = (x * MAP_EDGE_CELLS + z) as usize;
                if !walkable[idx] {
                    continue;
                }
                let here = top[idx];
                pinched[idx] = (-NAV_CLEARANCE_CELLS..=NAV_CLEARANCE_CELLS).any(|dx| {
                    (-NAV_CLEARANCE_CELLS..=NAV_CLEARANCE_CELLS).any(|dz| {
                        match nav_index(x + dx, z + dz) {
                            Some(n) => top[n] > here + NAV_STEP_CELLS,
                            None => true,
                        }
                    })
                });
            }
        }
        for (admitted, cramped) in walkable.iter_mut().zip(&pinched) {
            *admitted &= !cramped;
        }
        Self {
            walkable,
            waded,
            top,
        }
    }

    fn walkable(&self, x: i32, z: i32) -> bool {
        nav_index(x, z).is_some_and(|idx| self.walkable[idx])
    }

    fn top(&self, x: i32, z: i32) -> i32 {
        nav_index(x, z).map_or(i32::MIN, |idx| self.top[idx])
    }

    /// A single walking step between adjacent columns: both standable, and the
    /// rise no more than one source cell in either direction.
    fn step_ok(&self, from: (i32, i32), to: (i32, i32)) -> bool {
        self.walkable(to.0, to.1)
            && (self.top(to.0, to.1) - self.top(from.0, from.1)).abs() <= NAV_STEP_CELLS
    }

    /// Keep only the entrance's actual connected walking surface before snapping
    /// anchors. A nearby isolated terrace is not a usable route destination.
    fn retain_connected_to(&mut self, start: (i32, i32)) {
        let mut connected = vec![false; self.walkable.len()];
        connected[nav_index(start.0, start.1).unwrap()] = true;
        let mut pending = vec![start];
        while let Some(from) = pending.pop() {
            for (dx, dz) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let to = (from.0 + dx, from.1 + dz);
                if !self.step_ok(from, to)
                    || (dx != 0
                        && dz != 0
                        && !(self.step_ok(from, (from.0 + dx, from.1))
                            && self.step_ok(from, (from.0, from.1 + dz))))
                {
                    continue;
                }
                let i = nav_index(to.0, to.1).unwrap();
                if !connected[i] {
                    connected[i] = true;
                    pending.push(to);
                }
            }
        }
        self.walkable = connected;
    }

    /// Nearest walkable column to a metre position; bounded ring search.
    fn snap(&self, x: f32, z: f32) -> Option<(i32, i32)> {
        let cx = (x / TERRAIN_CELL_M).floor() as i32;
        let cz = (z / TERRAIN_CELL_M).floor() as i32;
        if self.walkable(cx, cz) {
            return Some((cx, cz));
        }
        for ring in 1..=NAV_SNAP_CELLS {
            let mut best: Option<((i32, i32), i32)> = None;
            for d in -ring..=ring {
                for candidate in [
                    (cx + d, cz - ring),
                    (cx + d, cz + ring),
                    (cx - ring, cz + d),
                    (cx + ring, cz + d),
                ] {
                    if !self.walkable(candidate.0, candidate.1) {
                        continue;
                    }
                    let cost = (candidate.0 - cx).pow(2) + (candidate.1 - cz).pow(2);
                    if best.is_none_or(|(_, b)| cost < b) {
                        best = Some((candidate, cost));
                    }
                }
            }
            if let Some((cell, _)) = best {
                return Some(cell);
            }
        }
        None
    }

    /// Bounded A* over walking edges. Eight-connected; a diagonal is admitted
    /// only when both of its orthogonal neighbours are walkable, so a path can
    /// never squeeze through a diagonal seam the capsule cannot pass.
    fn path(&self, start: (i32, i32), goal: (i32, i32)) -> Option<Vec<(i32, i32)>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        if !self.walkable(start.0, start.1) || !self.walkable(goal.0, goal.1) {
            return None;
        }
        // Octile distance in the same fixed-point units as the edge costs, so
        // the heuristic stays admissible.
        let heuristic = |c: (i32, i32)| -> i64 {
            let dx = (c.0 - goal.0).abs() as i64;
            let dz = (c.1 - goal.1).abs() as i64;
            1_000 * (dx + dz) + (1_414 - 2_000) * dx.min(dz)
        };
        let cells = (MAP_EDGE_CELLS * MAP_EDGE_CELLS) as usize;
        let mut cost = vec![i64::MAX; cells];
        let mut from = vec![u32::MAX; cells];
        let mut heap: BinaryHeap<Reverse<(i64, u32)>> = BinaryHeap::new();
        let start_idx = nav_index(start.0, start.1)?;
        cost[start_idx] = 0;
        heap.push(Reverse((heuristic(start), start_idx as u32)));
        let mut expansions = 0usize;
        while let Some(Reverse((_, packed))) = heap.pop() {
            let idx = packed as usize;
            let (x, z) = (
                packed as i32 / MAP_EDGE_CELLS,
                packed as i32 % MAP_EDGE_CELLS,
            );
            if (x, z) == goal {
                let mut path = vec![(x, z)];
                let mut cursor = idx;
                while from[cursor] != u32::MAX {
                    cursor = from[cursor] as usize;
                    path.push((
                        cursor as i32 / MAP_EDGE_CELLS,
                        cursor as i32 % MAP_EDGE_CELLS,
                    ));
                }
                path.reverse();
                return Some(path);
            }
            expansions += 1;
            if expansions > NAV_MAX_EXPANSIONS {
                return None;
            }
            let here = cost[idx];
            for (dx, dz) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let next = (x + dx, z + dz);
                if !self.step_ok((x, z), next) {
                    continue;
                }
                let diagonal = dx != 0 && dz != 0;
                if diagonal
                    && !(self.step_ok((x, z), (x + dx, z)) && self.step_ok((x, z), (x, z + dz)))
                {
                    continue;
                }
                let Some(next_idx) = nav_index(next.0, next.1) else {
                    continue;
                };
                let mut edge = if diagonal { 1_414 } else { 1_000 };
                if self.waded[next_idx] {
                    edge += NAV_WADE_PENALTY;
                }
                let candidate = here + edge;
                if candidate < cost[next_idx] {
                    cost[next_idx] = candidate;
                    from[next_idx] = idx as u32;
                    heap.push(Reverse((candidate + heuristic(next), next_idx as u32)));
                }
            }
        }
        None
    }

    /// Can the character walk the straight line between two columns? The route
    /// is followed as straight segments between waypoints, so the *line* has to
    /// be walkable, not only the cells the search visited.
    fn straight_walk(&self, a: (i32, i32), b: (i32, i32)) -> bool {
        let ax = a.0 as f32 + 0.5;
        let az = a.1 as f32 + 0.5;
        let bx = b.0 as f32 + 0.5;
        let bz = b.1 as f32 + 0.5;
        let span = ((bx - ax).abs()).max((bz - az).abs());
        let samples = (span * 4.0).ceil().max(1.0) as usize;
        let mut previous = self.top(a.0, a.1);
        for step in 1..=samples {
            let t = step as f32 / samples as f32;
            let x = lerp(ax, bx, t).floor() as i32;
            let z = lerp(az, bz, t).floor() as i32;
            if !self.walkable(x, z) {
                return false;
            }
            let top = self.top(x, z);
            if (top - previous).abs() > NAV_STEP_CELLS {
                return false;
            }
            previous = top;
        }
        true
    }
}

/// Route waypoints in metres from a walked cell path: greedily merge cells the
/// character can cross in a straight line, then resample at [`ROUTE_STEP_M`].
fn waypoints(terrain: &Terrain, nav: &NavGrid, cells: &[(i32, i32)]) -> Vec<[f32; 3]> {
    let mut corners = Vec::new();
    let mut anchor = 0usize;
    corners.push(cells[0]);
    while anchor + 1 < cells.len() {
        let mut next = anchor + 1;
        let mut furthest = next;
        while next < cells.len() {
            if nav.straight_walk(cells[anchor], cells[next]) {
                furthest = next;
                next += 1;
            } else {
                break;
            }
        }
        corners.push(cells[furthest]);
        anchor = furthest;
    }
    let to_m = |c: (i32, i32)| {
        [
            (c.0 as f32 + 0.5) * TERRAIN_CELL_M,
            (c.1 as f32 + 0.5) * TERRAIN_CELL_M,
        ]
    };
    let mut out: Vec<[f32; 3]> = Vec::new();
    let push = |x: f32, z: f32, out: &mut Vec<[f32; 3]>| {
        let Some(height) = terrain.height_at_metres(x, z) else {
            return;
        };
        let point = [x, height, z];
        if out.last() != Some(&point) {
            out.push(point);
        }
    };
    for pair in corners.windows(2) {
        let a = to_m(pair[0]);
        let b = to_m(pair[1]);
        let len = dist2(a[0], a[1], b[0], b[1]);
        let steps = (len / ROUTE_STEP_M).ceil().max(1.0) as usize;
        for step in 0..steps {
            let t = step as f32 / steps as f32;
            push(lerp(a[0], b[0], t), lerp(a[1], b[1], t), &mut out);
        }
    }
    if let Some(last) = corners.last().copied() {
        let p = to_m(last);
        push(p[0], p[1], &mut out);
    }
    out
}

/// Builds a route that is actually connected on the terrain: each spine anchor
/// is snapped onto the walking grid, and consecutive anchors are joined by a
/// searched walking path rather than a straight line of independent samples.
/// Anchors snap onto the entrance's connected surface. Any missing or unreachable
/// anchor is an explicit generation failure, never a silently shortened route.
fn densify(terrain: &Terrain, spine: &[[f32; 2]], close: bool) -> (Vec<[f32; 3]>, usize) {
    let mut nav = NavGrid::build(terrain);
    let Some(start) = spine.first().and_then(|p| nav.snap(p[0], p[1])) else {
        return (Vec::new(), spine.len());
    };
    nav.retain_connected_to(start);
    let mut anchors: Vec<(i32, i32)> = Vec::new();
    let mut missing = 0;
    for point in spine {
        match nav.snap(point[0], point[1]) {
            Some(cell) => {
                if anchors.last() != Some(&cell) {
                    anchors.push(cell);
                }
            }
            None => missing += 1,
        }
    }
    if close {
        if let Some(first) = anchors.first().copied() {
            anchors.push(first);
        }
    }
    if anchors.len() < 2 {
        return (Vec::new(), missing + 1);
    }
    let mut unreachable: Vec<(i32, i32)> = Vec::new();
    let mut cells: Vec<(i32, i32)> = vec![anchors[0]];
    let mut current = anchors[0];
    for anchor in anchors.iter().skip(1) {
        if *anchor == current {
            continue;
        }
        let Some(path) = nav.path(current, *anchor) else {
            // A spine anchor the walking grid cannot reach is a real routing
            // failure, not something to paper over: the caller reports it.
            unreachable.push(*anchor);
            continue;
        };
        cells.extend(path.into_iter().skip(1));
        current = *anchor;
    }
    if cells.len() < 2 {
        return (Vec::new(), unreachable.len() + missing);
    }
    (
        waypoints(terrain, &nav, &cells),
        unreachable.len() + missing,
    )
}

fn landmarks(terrain: &Terrain) -> Vec<Landmark> {
    let at = |name: &'static str, kind: &'static str, x: f32, z: f32, lift: f32| Landmark {
        name,
        kind,
        position_m: [x, terrain.height_at_metres(x, z).unwrap_or(0.0) + lift, z],
    };
    vec![
        at("basin", "water", BASIN_CENTRE_M[0], BASIN_CENTRE_M[1], 0.0),
        at("creek_mouth", "water", 63.0, 80.0, 0.0),
        at("creek_head", "water", 42.0, 10.0, 0.0),
        at("fungal_grove", "grove", 42.0, 62.0, 0.0),
        at("destruction_clearing", "clearing", 92.0, 58.0, 0.0),
        at("high_viewpoint", "viewpoint", 106.0, 30.0, 0.0),
        at("south_saddle", "pass", 64.0, 112.0, 0.0),
        Landmark {
            name: "west_rock_shelter",
            kind: "cavity",
            position_m: [CAVITIES[0].0[0], CAVITIES[0].0[1], CAVITIES[0].0[2]],
        },
        Landmark {
            name: "terrace_undercut",
            kind: "cavity",
            position_m: [CAVITIES[1].0[0], CAVITIES[1].0[1], CAVITIES[1].0[2]],
        },
        Landmark {
            name: "west_slope_undercut",
            kind: "cavity",
            position_m: [CAVITIES[2].0[0], CAVITIES[2].0[1], CAVITIES[2].0[2]],
        },
    ]
}

// ---------------------------------------------------------------------------
// Terrain voxel assembly
// ---------------------------------------------------------------------------

/// Shared fully solid deep-base prototype ids. These are genuinely dense
/// 32 x 32 x 16 volumes of real material, reused by transform.
pub const BASE_ROCK_ID: &str = "showcase_base_rock";
pub const BASE_SOIL_ID: &str = "showcase_base_soil";

fn solid_base(id: &str, fill: u8) -> Result<DetailVolume> {
    let mut volume = DetailVolume::new(id, Scale::new(TERRAIN_CELL_M)?);
    for x in 0..TILE_CELLS {
        for y in 0..BAND_CELLS {
            for z in 0..TILE_CELLS {
                volume.set([x, y, z], fill)?;
            }
        }
    }
    Ok(volume)
}

struct TerrainBuild {
    shell_prototypes: usize,
    base_instances: usize,
    overhang_columns: usize,
}

fn build_terrain_voxels(scene: &mut DetailScene, terrain: &Terrain) -> Result<TerrainBuild> {
    scene.add_prototype(solid_base(BASE_ROCK_ID, material::BANK_STONE)?)?;
    scene.add_prototype(solid_base(BASE_SOIL_ID, material::DETAIL_SOIL)?)?;

    let mut build = TerrainBuild {
        shell_prototypes: 0,
        base_instances: 0,
        overhang_columns: 0,
    };

    // Per-column overhang state for the tile currently being assembled. A
    // column belongs to exactly one tile, and bands are emitted bottom-up, so
    // this state carries across band boundaries and each (x, z) column is
    // counted at most once. The previous per-band reset both double counted
    // columns and missed roofs that started in the next band.
    let mut seen_air = vec![false; (TILE_CELLS * TILE_CELLS) as usize];
    let mut flagged = vec![false; (TILE_CELLS * TILE_CELLS) as usize];

    for tile_x in 0..TILES_PER_EDGE {
        for tile_z in 0..TILES_PER_EDGE {
            seen_air.fill(false);
            flagged.fill(false);
            let (mut min_top, mut max_top) = (i32::MAX, i32::MIN);
            for lx in 0..TILE_CELLS {
                for lz in 0..TILE_CELLS {
                    let idx = ((tile_x * TILE_CELLS + lx) * MAP_EDGE_CELLS
                        + tile_z * TILE_CELLS
                        + lz) as usize;
                    min_top = min_top.min(terrain.top[idx]);
                    max_top = max_top.max(terrain.top[idx].max(terrain.water_top[idx]));
                }
            }
            // The shell must start below the lowest surface in the tile and
            // below any carved cavity, so a shared solid base band is never
            // placed where real shape exists.
            let mut shell_floor = min_top - 8;
            if let Some(cavity) = tile_cavity_floor_cell(tile_x, tile_z) {
                shell_floor = shell_floor.min(cavity);
            }
            let shell_band = (shell_floor.max(0)) / BAND_CELLS;

            // Shared fully solid deep base bands.
            for band in 0..shell_band {
                let prototype = if band + 1 == shell_band {
                    BASE_SOIL_ID
                } else {
                    BASE_ROCK_ID
                };
                scene.place(
                    format!("base_{tile_x}_{tile_z}_{band}"),
                    prototype,
                    Transform::new(
                        [
                            (tile_x * TILE_CELLS) as f32 * TERRAIN_CELL_M,
                            (band * BAND_CELLS) as f32 * TERRAIN_CELL_M,
                            (tile_z * TILE_CELLS) as f32 * TERRAIN_CELL_M,
                        ],
                        Yaw::Deg0,
                    )?,
                )?;
                build.base_instances += 1;
            }

            // Unique surface/water/cavity shell bands.
            let top_band = max_top / BAND_CELLS;
            for band in shell_band..=top_band {
                let id = format!("shell_{tile_x}_{tile_z}_{band}");
                let mut volume = DetailVolume::new(&id, Scale::new(TERRAIN_CELL_M)?);
                let y0 = band * BAND_CELLS;
                let mut occupied = 0usize;
                for lx in 0..TILE_CELLS {
                    let gx = tile_x * TILE_CELLS + lx;
                    let wx = (gx as f32 + 0.5) * TERRAIN_CELL_M;
                    for lz in 0..TILE_CELLS {
                        let gz = tile_z * TILE_CELLS + lz;
                        let wz = (gz as f32 + 0.5) * TERRAIN_CELL_M;
                        let idx = (gx * MAP_EDGE_CELLS + gz) as usize;
                        let local = (lx * TILE_CELLS + lz) as usize;
                        for ly in 0..BAND_CELLS {
                            let y = y0 + ly;
                            let mut m = terrain.column_material(idx, y);
                            if m != material::AIR
                                && carved(wx, (y as f32 + 0.5) * TERRAIN_CELL_M, wz)
                            {
                                // A carved cell below the standing water surface
                                // floods, so the voxels keep agreeing with the
                                // water depth the surface query reports.
                                m = if y <= terrain.water_top[idx] {
                                    material::WATER
                                } else {
                                    material::AIR
                                };
                            }
                            if m == material::AIR {
                                seen_air[local] = true;
                            } else {
                                if seen_air[local]
                                    && material_policy(m) == MaterialPolicy::Collision
                                    && !flagged[local]
                                {
                                    // Genuine roof: collidable material above an
                                    // air gap in the same (x, z) column.
                                    flagged[local] = true;
                                    build.overhang_columns += 1;
                                }
                                volume.set([lx, ly, lz], m)?;
                                occupied += 1;
                            }
                        }
                    }
                }
                if occupied == 0 {
                    continue;
                }
                if occupied > MAX_PROTOTYPE_CELLS {
                    return Err(DetailError::BudgetExceeded("terrain band mesh preflight"));
                }
                scene.add_prototype(volume)?;
                build.shell_prototypes += 1;
                scene.place(
                    format!("i_{id}"),
                    &id,
                    Transform::new(
                        [
                            (tile_x * TILE_CELLS) as f32 * TERRAIN_CELL_M,
                            y0 as f32 * TERRAIN_CELL_M,
                            (tile_z * TILE_CELLS) as f32 * TERRAIN_CELL_M,
                        ],
                        Yaw::Deg0,
                    )?,
                )?;
            }
        }
    }
    Ok(build)
}

// ---------------------------------------------------------------------------
// Habitat-clustered flora scatter
// ---------------------------------------------------------------------------

/// Wet margins: reeds, ground rosettes, fronds and the segmented horsetail
/// spire, which is a marsh plant and is placed nowhere else.
const HABITAT_WATERSIDE: &[&str] = &[
    "reed_cluster",
    "rosette_groundcover",
    "fan_frond",
    "horsetail",
];
/// Sheltered low grove: the three cap mushrooms, fronds, the stump-rooted
/// bracket fungus and the twisted woody shrub.
const HABITAT_GROVE: &[&str] = &[
    "parasol_mushroom",
    "funnel_mushroom",
    "clustered_mushroom",
    "fan_frond",
    "bracket_fungus",
    "twisted_shrub",
];
/// Drier slopes and terraces: hardy clusters, groundcover, funnels, shrubs and
/// bracket fungus on its own stump.
const HABITAT_SLOPE: &[&str] = &[
    "clustered_mushroom",
    "rosette_groundcover",
    "funnel_mushroom",
    "twisted_shrub",
    "bracket_fungus",
];

/// Shallow-water archetype. The lily's pads sit at local `y = 2` cells of
/// 12.5 cm, i.e. 0.25 m above its rooted origin, so it is only truthful in a
/// column whose standing water is exactly one 25 cm cell deep: the rhizome at
/// `y = 0` is then submerged and the pads float at the water surface.
pub const SPECIES_LILY: &str = "marsh_lily";
pub const LILY_PAD_HEIGHT_M: f32 = 0.25;
/// Accepted standing-water depth window for lily placement, in metres.
const LILY_MIN_DEPTH_M: f32 = 0.2;
const LILY_MAX_DEPTH_M: f32 = 0.3;
/// Column stride of the shallow-water sweep, in 25 cm cells (50 cm).
const LILY_STRIDE_CELLS: i32 = 2;

/// Habitat clusters attempted. Bounded, deterministic, and independent of how
/// many succeed; the acceptance thresholds are checked on the finished scene.
const CLUSTER_ATTEMPTS: usize = 1_100;
const CLUSTER_MIN_PLANTS: usize = 14;
const CLUSTER_PLANT_SPREAD: usize = 38;
/// Foot occupancy grid pitch in metres; keeps feet from stacking.
const FOOT_PITCH_M: f32 = 0.375;

const YAWS: [Yaw; 4] = [Yaw::Deg0, Yaw::Deg90, Yaw::Deg180, Yaw::Deg270];

struct Scatter {
    instances: usize,
    expanded_cells: usize,
    species: BTreeMap<String, usize>,
}

/// True when a plant of horizontal source radius `radius_m` placed at `(x, z)`
/// would reach inside the protected corridor along any route segment.
fn route_clearance(route: &[[f32; 3]], x: f32, z: f32, radius_m: f32) -> bool {
    let limit = ROUTE_PLAYER_CLEARANCE_M + radius_m;
    if route.len() == 1 {
        return dist2(route[0][0], route[0][2], x, z) < limit;
    }
    route.windows(2).any(|pair| {
        let [a, b] = [pair[0], pair[1]];
        let dx = b[0] - a[0];
        let dz = b[2] - a[2];
        let length_squared = dx * dx + dz * dz;
        let t = if length_squared > 0. {
            (((x - a[0]) * dx + (z - a[2]) * dz) / length_squared).clamp(0., 1.)
        } else {
            0.
        };
        dist2(a[0] + t * dx, a[2] + t * dz, x, z) < limit
    })
}

fn place_flora(
    scene: &mut DetailScene,
    terrain: &Terrain,
    routes: &[&[[f32; 3]]],
    seed: u64,
) -> Result<Scatter> {
    let mut prototype_cells: BTreeMap<&str, usize> = BTreeMap::new();
    let mut radii: BTreeMap<&str, f32> = BTreeMap::new();
    for id in SHOWCASE_SPECIES {
        debug_assert!(
            FLORA_SPECIES.contains(&id)
                || matches!(
                    id,
                    "bracket_fungus" | "twisted_shrub" | "horsetail" | "marsh_lily"
                )
        );
        let volume = showcase_prototype(id)?;
        prototype_cells.insert(id, volume.occupied_cells());
        radii.insert(id, source_radius_m(&volume));
        scene.add_prototype(volume)?;
    }
    // Keep routes separate: joining their arrays would invent a corridor
    // segment between the end of one route and the start of the other.
    let pitch_cells = (MAP_EDGE_M / FOOT_PITCH_M).ceil() as usize;
    let mut taken = vec![false; pitch_cells * pitch_cells];
    let mut scatter = Scatter {
        instances: 0,
        expanded_cells: 0,
        species: BTreeMap::new(),
    };

    for cluster in 0..CLUSTER_ATTEMPTS {
        let k = cluster as i32;
        let cx = 5.0 + unit(seed ^ 0x11, k, 1) * (MAP_EDGE_M - 10.0);
        let cz = 5.0 + unit(seed ^ 0x12, k, 2) * (MAP_EDGE_M - 10.0);
        let Some(centre) = terrain.surface_at_metres(cx, cz) else {
            continue;
        };
        let near_water = (0..8).any(|i| {
            let angle = i as f32 * std::f32::consts::TAU / 8.0;
            terrain
                .surface_at_metres(cx + angle.cos() * 3.0, cz + angle.sin() * 3.0)
                .is_some_and(|p| p.water_depth_m > 0.0)
        });
        let palette: &[&str] = if centre.water_depth_m > 0.0 {
            continue;
        } else if near_water {
            HABITAT_WATERSIDE
        } else if centre.height_m < 22.0 && centre.slope < 0.6 {
            HABITAT_GROVE
        } else if centre.slope < 1.1 {
            HABITAT_SLOPE
        } else {
            continue;
        };
        let radius = 3.0 + unit(seed ^ 0x13, k, 3) * 5.0;
        let plants =
            CLUSTER_MIN_PLANTS + (hash2(seed ^ 0x14, k, 4) as usize % CLUSTER_PLANT_SPREAD);
        for plant in 0..plants {
            let p = plant as i32;
            let u = unit(seed ^ 0x21, k, p);
            let angle = unit(seed ^ 0x22, k, p) * std::f32::consts::TAU;
            let r = radius * u.sqrt();
            let x = cx + angle.cos() * r;
            let z = cz + angle.sin() * r;
            let Some(surface) = terrain.surface_at_metres(x, z) else {
                continue;
            };
            if surface.water_depth_m > 0.0 || surface.slope > 0.85 || surface.overhung {
                continue;
            }
            if surface.material != material::MOSS_TURF && surface.material != material::DETAIL_SOIL
            {
                continue;
            }
            let species = palette[(hash2(seed ^ 0x23, k, p) as usize) % palette.len()];
            if routes
                .iter()
                .any(|route| route_clearance(route, x, z, radii[species]))
            {
                continue;
            }
            let gx = (x / FOOT_PITCH_M).floor() as usize;
            let gz = (z / FOOT_PITCH_M).floor() as usize;
            if gx >= pitch_cells || gz >= pitch_cells {
                continue;
            }
            let slot = gx * pitch_cells + gz;
            if taken[slot] {
                continue;
            }
            taken[slot] = true;
            let yaw = YAWS[(hash2(seed ^ 0x24, k, p) as usize) % YAWS.len()];
            let transform = Transform::new([x, surface.height_m, z], yaw)?;
            scene.place(format!("flora_{cluster}_{plant}"), species, transform)?;
            scatter.instances += 1;
            scatter.expanded_cells += prototype_cells[species];
            *scatter.species.entry(species.to_string()).or_insert(0) += 1;
        }
    }

    // Shallow-water sweep for the lily. A random scatter over the whole map
    // almost never lands in the narrow one-cell-deep shoreline band, so the
    // band is swept directly and thinned deterministically.
    let lily_radius = radii[SPECIES_LILY];
    let mut placed_lilies = 0usize;
    for xi in (0..MAP_EDGE_CELLS).step_by(LILY_STRIDE_CELLS as usize) {
        let x = (xi as f32 + 0.5) * TERRAIN_CELL_M;
        for zi in (0..MAP_EDGE_CELLS).step_by(LILY_STRIDE_CELLS as usize) {
            let z = (zi as f32 + 0.5) * TERRAIN_CELL_M;
            let Some(surface) = terrain.surface_at_metres(x, z) else {
                continue;
            };
            // Truthful shallow water only: pads land on the water surface and
            // the rhizome stays submerged. Dry ground and deep water are both
            // rejected, so no lily ever floats over dry land.
            if surface.water_depth_m < LILY_MIN_DEPTH_M
                || surface.water_depth_m > LILY_MAX_DEPTH_M
                || surface.overhung
            {
                continue;
            }
            if !hash2(seed ^ 0x31, xi, zi).is_multiple_of(2) {
                continue;
            }
            if routes
                .iter()
                .any(|route| route_clearance(route, x, z, lily_radius))
            {
                continue;
            }
            let gx = (x / FOOT_PITCH_M).floor() as usize;
            let gz = (z / FOOT_PITCH_M).floor() as usize;
            if gx >= pitch_cells || gz >= pitch_cells || taken[gx * pitch_cells + gz] {
                continue;
            }
            taken[gx * pitch_cells + gz] = true;
            let yaw = YAWS[(hash2(seed ^ 0x32, xi, zi) as usize) % YAWS.len()];
            let transform = Transform::new([x, surface.height_m, z], yaw)?;
            scene.place(format!("lily_{xi}_{zi}"), SPECIES_LILY, transform)?;
            placed_lilies += 1;
            scatter.instances += 1;
            scatter.expanded_cells += prototype_cells[SPECIES_LILY];
            *scatter.species.entry(SPECIES_LILY.to_string()).or_insert(0) += 1;
        }
    }
    if placed_lilies == 0 {
        return Err(DetailError::BudgetExceeded(
            "no shallow water for marsh lily",
        ));
    }
    Ok(scatter)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Builds the complete showcase map for `seed`.
///
/// Every failure is explicit: budgets, prototype mesh preflight and the
/// documented content thresholds all return [`DetailError`] rather than a
/// quietly reduced scene.
pub fn build_showcase(seed: u64) -> Result<Showcase> {
    let terrain = Terrain::generate(seed)?;
    let mut scene = DetailScene::new();
    let build = build_terrain_voxels(&mut scene, &terrain)?;

    let (route, route_unreachable) = densify(&terrain, ROUTE_SPINE, true);
    let (elevated_route, elevated_unreachable) = densify(&terrain, &ELEVATED_SPINE, false);
    if route_unreachable > 0 {
        return Err(DetailError::BudgetExceeded(
            "ground spine anchor unreachable on the walking grid",
        ));
    }
    if elevated_unreachable > 0 {
        return Err(DetailError::BudgetExceeded(
            "elevated spine anchor unreachable on the walking grid",
        ));
    }
    if route.len() < 2 {
        return Err(DetailError::BudgetExceeded("route left the map"));
    }
    if elevated_route.len() < 2 {
        return Err(DetailError::BudgetExceeded("elevated route left the map"));
    }
    let scatter = place_flora(&mut scene, &terrain, &[&route, &elevated_route], seed)?;
    if scatter.species.len() != SHOWCASE_SPECIES.len() {
        return Err(DetailError::BudgetExceeded(
            "an archetype was not placed anywhere",
        ));
    }

    let counts = scene.counts();
    if counts.expanded_occupied_cells < SHOWCASE_EXPANDED_CELLS_MIN {
        return Err(DetailError::BudgetExceeded(
            "showcase expanded occupied cell target",
        ));
    }
    if scatter.expanded_cells < SHOWCASE_FLORA_CELLS_MIN {
        return Err(DetailError::BudgetExceeded("showcase flora cell target"));
    }
    if scatter.instances < SHOWCASE_PLANTS_MIN {
        return Err(DetailError::BudgetExceeded("showcase plant count target"));
    }

    // Per-class accounting straight from the authoritative scene.
    let mut classes: BTreeMap<String, ClassCounts> = BTreeMap::new();
    let mut largest_prototype_cells = 0usize;
    for id in scene.prototype_ids() {
        let volume = scene.prototype(&id).expect("listed prototype");
        let cells = volume.occupied_cells();
        largest_prototype_cells = largest_prototype_cells.max(cells);
        let entry = classes.entry(showcase_class(&id).to_string()).or_default();
        entry.prototypes += 1;
        entry.unique_stored_cells += cells;
    }
    let mut water_expanded_cells = 0usize;
    let mut terrain_expanded_cells = 0usize;
    for draw in scene.draws() {
        let entry = classes
            .entry(showcase_class(&draw.prototype).to_string())
            .or_default();
        entry.instances += 1;
        entry.expanded_cells += draw.occupied_cells;
        if showcase_class(&draw.prototype) == "terrain" {
            terrain_expanded_cells += draw.occupied_cells;
        }
    }
    for id in scene.prototype_ids() {
        if showcase_class(&id) != "terrain" {
            continue;
        }
        let volume = scene.prototype(&id).expect("listed prototype");
        let water = volume
            .iter_cells()
            .filter(|(_, m)| material_policy(*m) == MaterialPolicy::Liquid)
            .count();
        if water > 0 {
            // Terrain prototypes are placed exactly once each, except the two
            // shared solid bases, which contain no water.
            water_expanded_cells += water;
        }
    }

    let spawn = terrain
        .surface_at_metres(ROUTE_SPINE[0][0], ROUTE_SPINE[0][1])
        .ok_or(DetailError::BudgetExceeded("spawn outside the map"))?;
    let spawn_eye = [
        ROUTE_SPINE[0][0],
        spawn.height_m + EYE_HEIGHT_M,
        ROUTE_SPINE[0][1],
    ];

    let route_length_m = polyline_length(&route);
    let manifest = ContentManifest {
        generator_version: SHOWCASE_GENERATOR_VERSION,
        seed,
        classes,
        species_instances: scatter.species,
        terrain_shell_prototypes: build.shell_prototypes,
        terrain_base_prototypes: 2,
        terrain_base_instances: build.base_instances,
        flora_instances: scatter.instances,
        flora_expanded_cells: scatter.expanded_cells,
        terrain_expanded_cells,
        water_expanded_cells,
        overhang_columns: build.overhang_columns,
        route_length_m,
        route_walk_seconds: route_length_m / WALK_SPEED_M_S,
        largest_prototype_cells,
    };

    let landmarks = landmarks(&terrain);
    Ok(Showcase {
        scene,
        terrain,
        spawn_eye,
        route,
        elevated_route,
        landmarks,
        manifest,
    })
}

/// Content class of a showcase prototype id: terrain, or the flora class from
/// the existing catalogue helper.
pub fn showcase_class(prototype_id: &str) -> &'static str {
    match prototype_id {
        _ if prototype_id.starts_with("shell_")
            || prototype_id == BASE_ROCK_ID
            || prototype_id == BASE_SOIL_ID =>
        {
            "terrain"
        }
        // The bracket fungus is a fungus with its own woody stump; the stump is
        // collidable, so it is accounted with the collidable fungi.
        "bracket_fungus" => "flora-fungus",
        // The twisted shrub is the only woody archetype: a collidable trunk and
        // branches with decorative crowns. It is reported separately so a
        // manifest never implies a mushroom count that includes wood.
        "twisted_shrub" => "flora-woody",
        "horsetail" | "marsh_lily" => "flora-decorative",
        _ => flora_class(prototype_id),
    }
}

/// Order-independent composition hash of the authoritative placement: every
/// instance's prototype, transform and prototype cell count. Two runs that agree
/// here generated the same map.
pub fn composition_hash(showcase: &Showcase) -> u64 {
    let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
    let mix = |acc: &mut u64, value: u64| {
        *acc ^= value;
        *acc = acc.wrapping_mul(0x1000_0000_01b3);
    };
    for draw in showcase.scene.draws() {
        for byte in draw.instance.as_bytes() {
            mix(&mut acc, u64::from(*byte));
        }
        for byte in draw.prototype.as_bytes() {
            mix(&mut acc, u64::from(*byte));
        }
        for axis in draw.transform.translation_m {
            mix(&mut acc, u64::from(axis.to_bits()));
        }
        mix(&mut acc, draw.transform.yaw as u64);
        mix(&mut acc, draw.occupied_cells as u64);
    }
    for id in showcase.scene.prototype_ids() {
        let volume = showcase.scene.prototype(&id).expect("listed prototype");
        mix(&mut acc, volume.occupied_cells() as u64);
        mix(&mut acc, volume.source_bytes() as u64);
    }
    acc
}
