//! Original P03 flora catalogue and dense representative tile.
//!
//! Prototype geometry uses integer construction; placement uses deterministic
//! hash ordering and metre-space transforms. The existing parasol mushroom and
//! terrain tile fixtures are reused unchanged; the bracket/shelf species is
//! RESERVED and is not implemented here.
//!
//! Scales are explicit per prototype: fungi at 6.25 cm cells, leafy flora at
//! 12.5 cm cells. Every prototype stays far below the 33,288-cell conservative
//! mesh/coarsen preflight cap so all three LODs fit the public budgets.

use crate::{
    fixtures::{parasol_mushroom, terrain_detail_tile},
    material, material_policy, DetailError, DetailScene, DetailVolume, MaterialPolicy, Result,
    Scale, Transform, Yaw, SCALE_FINE_M, SCALE_TILE_M, TILE_EDGE_CELLS, TILE_WATER_LEVEL_CELLS,
};
use std::collections::BTreeSet;

/// Flora catalogue version. Fixtures remain at their own version; both are
/// recorded in exported manifests.
pub const FLORA_GENERATOR_VERSION: u32 = 2;
/// Canonical dense-tile seed. Placement searches are bounded and fail
/// explicitly if this seed cannot meet the density thresholds.
pub const FLORA_CANONICAL_SEED: u64 = 20260908;
/// Explicit fine scale for fungal prototypes (6.25 cm cells).
pub const FLORA_FUNGUS_SCALE_M: f32 = SCALE_FINE_M;
/// Explicit scale for leafy prototypes (12.5 cm cells).
pub const FLORA_LEAF_SCALE_M: f32 = 0.125;
/// Minimum vegetation (non-terrain) instances in [`dense_tile`].
pub const DENSE_VEGETATION_MIN: usize = 64;
/// Minimum instance-expanded flora cells (terrain excluded) in [`dense_tile`].
pub const DENSE_FLORA_CELLS_MIN: usize = 40_000;
/// Minimum distinct flora types placed in [`dense_tile`].
pub const DENSE_TYPES_MIN: usize = 5;
/// North-south origin exclusion: tile columns `x in [-4, 4)` carry no placements.
pub const CORRIDOR_TILE_X: (i32, i32) = (-4, 4);

/// Flora prototype IDs placed by [`dense_tile`] (terrain excluded).
pub const FLORA_SPECIES: [&str; 6] = [
    "parasol_mushroom",
    "funnel_mushroom",
    "clustered_mushroom",
    "fan_frond",
    "reed_cluster",
    "rosette_groundcover",
];

fn hash3(seed: u64, x: i32, z: i32) -> u64 {
    let mut value = seed
        ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (z as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Quarter-turn yaw of a local-metre offset (mirrors the documented
/// `Yaw::rotate` formula, which is crate-private).
fn yaw_offset(yaw: Yaw, p: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = p;
    match yaw {
        Yaw::Deg0 => [x, y, z],
        Yaw::Deg90 => [z, y, -x],
        Yaw::Deg180 => [-x, y, -z],
        Yaw::Deg270 => [-z, y, x],
    }
}

/// Broad classification for manifests: terrain, collidable fungus or
/// decorative leafy flora.
pub fn flora_class(prototype_id: &str) -> &'static str {
    match prototype_id {
        "terrain_tile_16m" => "terrain",
        "parasol_mushroom" | "funnel_mushroom" | "clustered_mushroom" => "flora-fungus",
        _ => "flora-decorative",
    }
}

/// Nominal canopy radius in metres, used only for placement spacing so visual
/// layers may overlap while feet stay disjoint.
fn canopy_radius_m(prototype_id: &str) -> f32 {
    match prototype_id {
        "parasol_mushroom" | "funnel_mushroom" => 0.6,
        "clustered_mushroom" => 0.75,
        "fan_frond" => 1.0,
        "reed_cluster" => 0.45,
        "rosette_groundcover" => 0.5,
        _ => 0.5,
    }
}

// ---------------------------------------------------------------------------
// Prototypes
// ---------------------------------------------------------------------------

/// Twelve integer blade directions scaled by 16 (funnel gills).
const FUNNEL_BLADES: [[i32; 2]; 12] = [
    [16, 0],
    [14, 8],
    [8, 14],
    [0, 16],
    [-8, 14],
    [-14, 8],
    [-16, 0],
    [-14, -8],
    [-8, -14],
    [0, -16],
    [8, -14],
    [14, -8],
];

/// Original funnel mushroom at 6.25 cm cells: hollow concave pit through the
/// cap centre, rolled/ribbed rim band, radial underside gills and a connected
/// tapering stipe. Foot radius 3 cells.
pub fn funnel_mushroom(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(FLORA_FUNGUS_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    // Stipe: compact root disc at y = 0 (the only ground-contact layer), a
    // flare just above it, slim waist, swell under the cap.
    for y in 0..=11 {
        let radius = match y {
            0 => 2,
            1..=2 => 3,
            3 => 2,
            4..=9 => 1,
            _ => 2,
        };
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x * x + z * z <= radius * radius {
                    volume.set([x, y, z], material::FLORA_FUNNEL_STIPE)?;
                }
            }
        }
    }
    volume.set([2, 7, 0], material::FLORA_FUNNEL_STIPE)?;
    // Collar joins stipe to cap flesh.
    for x in -2..=2 {
        for z in -2..=2 {
            if x * x + z * z <= 4 {
                volume.set([x, 12, z], material::FLORA_FUNNEL_STIPE)?;
            }
        }
    }
    // Radial gills under the cap, flesh directly above at y = 13.
    for blade in FUNNEL_BLADES {
        for r in 3..=6 {
            let cell = [blade[0] * r / 16, 12, blade[1] * r / 16];
            if volume.get(cell) == material::AIR {
                volume.set(cell, material::FLORA_FUNNEL_GILL)?;
            }
        }
    }
    // Funnel cap: annuli with a pit that widens upward (actual holes, y 13..18).
    for (y, outer, inner) in [
        (13i32, 7i32, 1i32),
        (14, 8, 2),
        (15, 9, 3),
        (16, 9, 4),
        (17, 8, 5),
        (18, 6, 5),
    ] {
        for x in -outer..=outer {
            for z in -outer..=outer {
                let r2 = x * x + z * z;
                if r2 <= outer * outer && r2 > inner * inner {
                    let chosen = if (15..=16).contains(&y) && r2 > 49 {
                        // Rolled rim band: ribbed ring, every fourth cell cap flesh.
                        if (x.abs() + z.abs()) % 4 == 0 {
                            material::FLORA_FUNNEL_CAP
                        } else {
                            material::FLORA_FUNNEL_RIM
                        }
                    } else if y == 17 && (26..=30).contains(&r2) {
                        // Restrained luminous pit-wall accent. It is fungal
                        // flesh, so it uses the collidable fungus accent ID
                        // rather than the decorative leaf accent: colour must
                        // never punch a hole in a gameplay body.
                        material::FLORA_FUNGUS_LUMEN
                    } else if y >= 17 && (x * 5 - z * 3) % 7 == 0 {
                        // Deterministic vein zoning on the upper funnel.
                        material::FLORA_FUNNEL_RIM
                    } else {
                        material::FLORA_FUNNEL_CAP
                    };
                    volume.set([x, y, z], chosen)?;
                }
            }
        }
    }
    Ok(volume)
}

/// Eight integer gill directions scaled by 8 (clustered caps).
const CLUSTER_BLADES: [[i32; 2]; 8] = [
    [8, 0],
    [6, 6],
    [0, 8],
    [-6, 6],
    [-8, 0],
    [-6, -6],
    [0, -8],
    [6, -6],
];

/// Four stems: (base, stipe height, cap radius, lean). Distinct heights, radii
/// and one leaning stem give deterministic bounded variation.
const CLUSTER_STEMS: [([i32; 2], i32, i32, i32); 4] = [
    ([-5, -4], 11, 5, 0),
    ([4, -5], 14, 6, 1),
    ([-4, 5], 9, 4, 0),
    ([5, 4], 12, 5, -1),
];

/// Original clustered cap-and-stem mushroom at 6.25 cm cells: four distinct
/// legible caps on separate stipes joined by y = 0 root runners. Each cap has
/// a shaped crown, a rim ring and underside gills.
pub fn clustered_mushroom(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(FLORA_FUNGUS_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    // Compact connected root base: one disc at y = 0 is the whole ground
    // contact footprint. Stems rise from y = 1 and are tied back to the disc by
    // runners at y = 1, so the clump reads as one body growing from one root
    // mass instead of four separate plants sharing a bounding box.
    for x in -2..=2 {
        for z in -2..=2 {
            if x * x + z * z <= 4 {
                volume.set([x, 0, z], material::FLORA_CLUSTER_STIPE)?;
            }
        }
    }
    for (base, _, _, _) in CLUSTER_STEMS {
        let (mut cx, mut cz) = (0, 0);
        while cx != base[0] {
            cx += (base[0] - cx).signum();
            if volume.get([cx, 1, cz]) == material::AIR {
                volume.set([cx, 1, cz], material::FLORA_CLUSTER_STIPE)?;
            }
        }
        while cz != base[1] {
            cz += (base[1] - cz).signum();
            if volume.get([cx, 1, cz]) == material::AIR {
                volume.set([cx, 1, cz], material::FLORA_CLUSTER_STIPE)?;
            }
        }
        // Tie the runner back down to the root disc where it passes over it.
        if volume.get([0, 1, 0]) == material::AIR {
            volume.set([0, 1, 0], material::FLORA_CLUSTER_STIPE)?;
        }
    }
    for (base, height, cap_radius, lean) in CLUSTER_STEMS {
        // Stipe with flared foot; upper half shifts by `lean` (bounded variation).
        for y in 1..=height {
            let radius = if y <= 2 { 2 } else { 1 };
            let shift = if y * 2 > height { lean } else { 0 };
            for x in -radius..=radius {
                for z in -radius..=radius {
                    if x * x + z * z <= radius * radius {
                        volume.set(
                            [base[0] + shift + x, y, base[1] + z],
                            material::FLORA_CLUSTER_STIPE,
                        )?;
                    }
                }
            }
        }
        let centre = [base[0] + lean, base[1]];
        let collar = height + 1;
        // Collar plus radial gills under the cap flesh (cap base at collar + 1).
        for x in -2..=2 {
            for z in -2..=2 {
                if x * x + z * z <= 4 {
                    volume.set(
                        [centre[0] + x, collar, centre[1] + z],
                        material::FLORA_CLUSTER_STIPE,
                    )?;
                }
            }
        }
        for blade in CLUSTER_BLADES {
            for r in 2..cap_radius {
                let cell = [
                    centre[0] + blade[0] * r / 8,
                    collar,
                    centre[1] + blade[1] * r / 8,
                ];
                if volume.get(cell) == material::AIR {
                    volume.set(cell, material::FLORA_CLUSTER_GILL)?;
                }
            }
        }
        // Shaped crown: broad base disc with rim ring, stepped crown, umbo.
        let layers = [
            (collar + 1, cap_radius),
            (collar + 2, cap_radius - 2),
            (collar + 3, (cap_radius - 4).max(1)),
        ];
        for (y, radius) in layers {
            for x in -radius..=radius {
                for z in -radius..=radius {
                    let r2 = x * x + z * z;
                    if r2 <= radius * radius {
                        let chosen = if y == collar + 1 && r2 > (radius - 1) * (radius - 1) {
                            material::FLORA_CLUSTER_RIM
                        } else {
                            material::FLORA_CLUSTER_CAP
                        };
                        volume.set([centre[0] + x, y, centre[1] + z], chosen)?;
                    }
                }
            }
        }
        volume.set(
            [centre[0], collar + 4, centre[1]],
            material::FLORA_CLUSTER_CAP,
        )?;
    }
    Ok(volume)
}

/// Seven frond directions (unit-ish, scaled by 8) with per-frond lengths.
const FROND_DIRS: [[i32; 2]; 7] = [[8, 0], [6, 6], [0, 8], [-6, 6], [-8, 0], [-6, -6], [6, -6]];
const FROND_LEN: [i32; 7] = [12, 10, 13, 11, 12, 10, 11];

/// Set `to`, walking from `from` one axis at a time (x, then z, then y) and
/// filling every intermediate cell that is still air, so a rasterised stem,
/// rib or leaf path is six-connected instead of a chain of diagonal touches.
/// Cells that already hold geometry are left alone.
fn set_path(volume: &mut DetailVolume, from: [i32; 3], to: [i32; 3], m: u8) -> Result<()> {
    let mut cur = from;
    for axis in [0usize, 2, 1] {
        while cur[axis] != to[axis] {
            cur[axis] += (to[axis] - cur[axis]).signum();
            if volume.get(cur) == material::AIR {
                volume.set(cur, m)?;
            }
        }
    }
    Ok(())
}

/// Half-width of the blade flanking a rib at parameter `t` of `length`: narrow
/// at the sheath, widest across the middle third, tapering to a point. This is
/// what makes the prototype read as a fan/frond rather than a bundle of sticks.
fn frond_half_width(t: i32, length: i32) -> i32 {
    if t < 2 || t >= length {
        return 0;
    }
    let from_tip = length - t;
    match (t, from_tip) {
        (2..=3, _) => 1,
        (_, 1) => 1,
        (_, 2) => 2,
        _ => 3,
    }
}

/// Original fan frond at 12.5 cm cells (Decorative): a compact rhizome foot
/// (the only ground-contact layer) carrying seven arched ribs. Each rib is
/// flanked by a blade whose half-width swells across the middle and tapers to
/// the tip, with a rib-material side vein along the blade margin every third
/// row, so the silhouette is a fan of leaves, not a bundle of sticks.
pub fn fan_frond(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(FLORA_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    for x in -1..=1 {
        for z in -1..=1 {
            if x * x + z * z <= 1 {
                volume.set([x, 0, z], material::FLORA_FROND_STEM)?;
            }
        }
    }
    for x in -1..=1 {
        for z in -1..=1 {
            volume.set([x, 1, z], material::FLORA_FROND_STEM)?;
        }
    }
    for (index, dir) in FROND_DIRS.into_iter().enumerate() {
        let length = FROND_LEN[index];
        let perp = [-dir[1].signum(), dir[0].signum()];
        // The rib is walked cell by cell from the sheath outward: a step that
        // advances both the radius and the arch height would otherwise leave a
        // diagonal break, which is what split this prototype into 71 pieces.
        let mut rib_prev = [0, 1, 0];
        for t in 0..=length {
            let (px, pz) = (dir[0] * t / 8, dir[1] * t / 8);
            let y = 1 + t - t * t / length;
            let rib = [px, y, pz];
            set_path(&mut volume, rib_prev, rib, material::FLORA_FROND_RIB)?;
            rib_prev = rib;
            let width = frond_half_width(t, length);
            for sign in [1, -1] {
                // Blade rows also walk outward from the rib, so a diagonal
                // `perp` produces a solid blade rather than a diagonal thread.
                let mut prev = rib;
                for step in 1..=width {
                    // Margin vein every third row; interior of the blade is leaf.
                    let side_material = if step == width && t % 4 == 0 {
                        material::FLORA_FROND_RIB
                    } else {
                        material::FLORA_FROND_BLADE
                    };
                    let cell = [px + perp[0] * sign * step, y, pz + perp[1] * sign * step];
                    set_path(&mut volume, prev, cell, side_material)?;
                    prev = cell;
                }
            }
        }
    }
    Ok(volume)
}

/// Nine culm offsets (3 x 3 grid at 2-cell steps) with per-culm heights.
const REED_OFFSETS: [[i32; 2]; 9] = [
    [-2, -2],
    [0, -2],
    [2, -2],
    [-2, 0],
    [0, 0],
    [2, 0],
    [-2, 2],
    [0, 2],
    [2, 2],
];
const REED_HEIGHTS: [i32; 9] = [12, 15, 10, 14, 16, 11, 13, 9, 12];
const REED_LEAF_DIRS: [[i32; 2]; 9] = [
    [1, 0],
    [1, 1],
    [0, 1],
    [-1, 1],
    [1, -1],
    [-1, 0],
    [-1, -1],
    [0, -1],
    [1, 0],
];

/// Original reed cluster at 12.5 cm cells (Decorative): nine culms of varied
/// height on a small foot, angled leaves at third-points and plus-shaped plume
/// heads.
pub fn reed_cluster(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(FLORA_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    // Compact rhizome pad: the only ground-contact layer. Culms rise from it at
    // y = 1 and are joined to it by a runner, so the cluster is one rooted body
    // with a small footprint instead of nine separate stalks.
    for x in -1..=1 {
        for z in -1..=1 {
            volume.set([x, 0, z], material::FLORA_REED_STEM)?;
        }
    }
    for (index, off) in REED_OFFSETS.into_iter().enumerate() {
        let height = REED_HEIGHTS[index];
        let (mut cx, mut cz) = (0, 0);
        while cx != off[0] {
            cx += (off[0] - cx).signum();
            volume.set([cx, 1, cz], material::FLORA_REED_STEM)?;
        }
        while cz != off[1] {
            cz += (off[1] - cz).signum();
            volume.set([cx, 1, cz], material::FLORA_REED_STEM)?;
        }
        for y in 1..=height {
            volume.set([off[0], y, off[1]], material::FLORA_REED_STEM)?;
        }
        let dir = REED_LEAF_DIRS[index];
        for (third, base) in [height / 3, 2 * height / 3].into_iter().enumerate() {
            let flip = if third == 0 { 1 } else { -1 };
            // Walk the leaf out one axis step at a time so a diagonal leaf is a
            // connected arch off the culm, not a row of diagonally touching
            // cells that leaves the cluster in many pieces.
            let mut cell = [off[0], base, off[1]];
            for k in 1..=3 {
                if dir[0] != 0 {
                    cell[0] += dir[0] * flip;
                    volume.set(cell, material::FLORA_REED_LEAF)?;
                }
                if dir[1] != 0 {
                    cell[2] += dir[1] * flip;
                    volume.set(cell, material::FLORA_REED_LEAF)?;
                }
                if k % 2 == 0 {
                    cell[1] += 1;
                    volume.set(cell, material::FLORA_REED_LEAF)?;
                }
            }
        }
        for dy in 0..=1 {
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    if dx.abs() + dz.abs() <= 1 {
                        volume.set(
                            [off[0] + dx, height + 1 + dy, off[1] + dz],
                            material::FLORA_REED_PLUME,
                        )?;
                    }
                }
            }
        }
        volume.set([off[0], height + 3, off[1]], material::FLORA_LUMEN_DOT)?;
    }
    Ok(volume)
}

/// Eight compass rosette directions; outer leaves radiate flat then lift.
const ROSETTE_DIRS: [[i32; 2]; 8] = [
    [1, 0],
    [1, 1],
    [0, 1],
    [-1, 1],
    [-1, 0],
    [-1, -1],
    [0, -1],
    [1, -1],
];

/// Original low rosette/groundcover at 12.5 cm cells (Decorative): eight outer
/// leaves (accent-spotted tips on even leaves), an inner heart ring and a bud.
pub fn rosette_groundcover(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(FLORA_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    // Compact crown pad at y = 0 is the whole ground contact; the leaf whorl
    // sits on it at y = 1 and lifts at the tips.
    for x in -1..=1 {
        for z in -1..=1 {
            volume.set([x, 0, z], material::FLORA_ROSETTE_LEAF)?;
        }
    }
    for (index, dir) in ROSETTE_DIRS.into_iter().enumerate() {
        let perp = [-dir[1], dir[0]];
        // Diagonal leaves are walked outward one axis at a time; the flanking
        // half-row is walked off the midrib the same way. Placing those cells
        // directly left the diagonal leaves as isolated voxels.
        let mut midrib_prev = [0, 1, 0];
        for t in 0..=5 {
            let y = if t < 3 { 1 } else { 2 };
            let width = match t {
                0 => 1,
                1 | 2 => 2,
                _ => 1,
            };
            let tip = t == 5 && index % 2 == 0;
            let leaf = if tip {
                material::FLORA_ROSETTE_SPOT
            } else {
                material::FLORA_ROSETTE_LEAF
            };
            let midrib = [dir[0] * t, y, dir[1] * t];
            set_path(&mut volume, midrib_prev, midrib, leaf)?;
            if volume.get(midrib) == material::AIR {
                volume.set(midrib, leaf)?;
            }
            midrib_prev = midrib;
            if width > 1 {
                let cell = [midrib[0] + perp[0], y, midrib[2] + perp[1]];
                set_path(&mut volume, midrib, cell, leaf)?;
            }
        }
        // Inner heart leaf above the outer base, joined to the bud.
        let mut heart_prev = [0, 2, 0];
        for t in 0..=2 {
            let cell = [dir[0] * t, 2, dir[1] * t];
            set_path(&mut volume, heart_prev, cell, material::FLORA_ROSETTE_HEART)?;
            heart_prev = cell;
        }
    }
    for x in -1..=1 {
        for z in -1..=1 {
            if volume.get([x, 2, z]) == material::AIR {
                volume.set([x, 2, z], material::FLORA_ROSETTE_HEART)?;
            }
        }
    }
    for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
        volume.set([dx, 3, dz], material::FLORA_ROSETTE_HEART)?;
    }
    volume.set([0, 4, 0], material::FLORA_ROSETTE_HEART)?;
    Ok(volume)
}

/// Build one flora prototype by ID (parasol reuses the existing fixture).
pub fn flora_prototype(id: &str) -> Result<DetailVolume> {
    match id {
        "parasol_mushroom" => parasol_mushroom(id),
        "funnel_mushroom" => funnel_mushroom(id),
        "clustered_mushroom" => clustered_mushroom(id),
        "fan_frond" => fan_frond(id),
        "reed_cluster" => reed_cluster(id),
        "rosette_groundcover" => rosette_groundcover(id),
        _ => Err(DetailError::UnknownPrototype(id.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Dense representative tile
// ---------------------------------------------------------------------------

/// Precomputed tile tops: `(height_cell, material)` per tile column.
struct Tops {
    half: i32,
    cells: Vec<Option<(i32, u8)>>,
}

impl Tops {
    fn build(tile: &DetailVolume) -> Result<Self> {
        let (min, max) = tile
            .cell_bounds()
            .ok_or(DetailError::BudgetExceeded("empty terrain tile"))?;
        let half = TILE_EDGE_CELLS / 2;
        let mut cells = vec![None; (TILE_EDGE_CELLS * TILE_EDGE_CELLS) as usize];
        for x in -half..half {
            for z in -half..half {
                let mut top = None;
                for y in (min[1]..=max[1]).rev() {
                    let m = tile.get([x, y, z]);
                    if m != material::AIR {
                        top = Some((y, m));
                        break;
                    }
                }
                cells[((x + half) * TILE_EDGE_CELLS + (z + half)) as usize] = top;
            }
        }
        Ok(Self { half, cells })
    }

    fn get(&self, x: i32, z: i32) -> Option<(i32, u8)> {
        if x < -self.half || x >= self.half || z < -self.half || z >= self.half {
            return None;
        }
        self.cells[((x + self.half) * TILE_EDGE_CELLS + (z + self.half)) as usize]
    }
}

/// Verify the complete foot/contact footprint against authoritative source,
/// using the instance's ACTUAL world origin (`world_x`, `world_z` in metres),
/// so a fractional translation moves the sampled terrain columns.
///
/// Contact is the set of tile columns under the prototype's *bottom* cells
/// (local y = 0) only. A canopy, blade or plume bounding box is deliberately
/// NOT a ground footprint: requiring flat terrain under a 1.5 m frond canopy
/// rejects almost every column of a natural creek bank and produced the
/// exhausted placement search in the previous attempt. Every contact column
/// must be dry moss at the same top height, so no foot floats or sinks.
fn support_height(
    tops: &Tops,
    prototype: &DetailVolume,
    yaw: Yaw,
    world_x: f32,
    world_z: f32,
) -> Option<i32> {
    let tile_cell = SCALE_TILE_M;
    let cx = (world_x / tile_cell).floor() as i32;
    let cz = (world_z / tile_cell).floor() as i32;
    let (base, base_material) = tops.get(cx, cz)?;
    if base_material != material::MOSS_TURF || base <= TILE_WATER_LEVEL_CELLS {
        return None;
    }
    let fine = prototype.scale().metres();
    let mut contacts = BTreeSet::new();
    for (cell, _) in prototype.iter_cells() {
        if cell[1] != 0 {
            continue;
        }
        let local = [
            (cell[0] as f32 + 0.5) * fine,
            0.0,
            (cell[2] as f32 + 0.5) * fine,
        ];
        let rotated = yaw_offset(yaw, local);
        let wx = world_x + rotated[0];
        let wz = world_z + rotated[2];
        contacts.insert([
            (wx / tile_cell).floor() as i32,
            (wz / tile_cell).floor() as i32,
        ]);
    }
    if contacts.is_empty() {
        return None;
    }
    for [tx, tz] in &contacts {
        match tops.get(*tx, *tz) {
            Some((h, m)) if h == base && m == material::MOSS_TURF => {}
            _ => return None,
        }
    }
    Some(base)
}

/// True when ANY source cell of `prototype`, placed by `transform`, has its
/// centre inside a solid (non-water) terrain cell.
///
/// Foot contact at local `y = 0` is not sufficient: on a slope or a bank the
/// upper stipes and canopy of a grounded plant can still be driven into the
/// hillside. Water is ignored for above-root anatomy; the root contact check
/// still requires dry moss. This checks the whole source anatomy, not a bounding
/// box, and it does not require the canopy to sit over one ground height.
fn intersects_terrain(
    tile: &DetailVolume,
    prototype: &DetailVolume,
    transform: &Transform,
) -> bool {
    let fine = prototype.scale().metres();
    let tile_cell = SCALE_TILE_M;
    for (cell, _) in prototype.iter_cells() {
        let local = [
            (cell[0] as f32 + 0.5) * fine,
            (cell[1] as f32 + 0.5) * fine,
            (cell[2] as f32 + 0.5) * fine,
        ];
        let rotated = yaw_offset(transform.yaw, local);
        let world = [
            transform.translation_m[0] + rotated[0],
            transform.translation_m[1] + rotated[1],
            transform.translation_m[2] + rotated[2],
        ];
        let terrain = tile.get([
            (world[0] / tile_cell).floor() as i32,
            (world[1] / tile_cell).floor() as i32,
            (world[2] / tile_cell).floor() as i32,
        ]);
        if terrain != material::AIR && terrain != material::WATER {
            return true;
        }
    }
    false
}

/// Public support query for one placed instance: the derived support height
/// cell, its material name and the vertical gap in metres (0.0 when rooted).
///
/// Returns `None` when the instance is not acceptably placed, which now covers
/// two distinct cases: the local `y = 0` contact footprint is not fully carried
/// by dry moss at one height, OR some source cell of the plant intersects solid
/// terrain (see [`intersects_terrain`]). Both are reported as "unsupported"
/// because both mean the placement is not a legitimately rooted plant.
/// This fixture helper requires a 16 m, 0.25 m-cell terrain tile at identity
/// transform and flora whose bottom layer is local y = 0. It reports positive
/// vertical gaps rather than treating a raised instance as rooted. Clearance is
/// sampled at source voxel centres, not by general continuous collision tests.
pub fn instance_support(
    tile: &DetailVolume,
    prototype: &DetailVolume,
    prototype_id: &str,
    transform: &Transform,
) -> Option<(i32, &'static str, f32)> {
    transform.validate().ok()?;
    if tile.scale().metres() != SCALE_TILE_M || prototype.cell_bounds()?.0[1] != 0 {
        return None;
    }
    let tops = Tops::build(tile).ok()?;
    let tile_cell = SCALE_TILE_M;
    let _ = prototype_id;
    let base = support_height(
        &tops,
        prototype,
        transform.yaw,
        transform.translation_m[0],
        transform.translation_m[2],
    )?;
    if intersects_terrain(tile, prototype, transform) {
        return None;
    }
    let cx = (transform.translation_m[0] / tile_cell).floor() as i32;
    let cz = (transform.translation_m[2] / tile_cell).floor() as i32;
    let (_, material) = tops.get(cx, cz)?;
    let gap = transform.translation_m[1] - (base + 1) as f32 * tile_cell;
    Some((base, crate::material_name(material), gap))
}

const DENSE_YAWS: [Yaw; 4] = [Yaw::Deg0, Yaw::Deg90, Yaw::Deg180, Yaw::Deg270];

/// Per-species placement targets and habitat rules. Targets are upper bounds on
/// one bounded pass, not hard quotas: the acceptance thresholds are the global
/// [`DENSE_VEGETATION_MIN`] / [`DENSE_FLORA_CELLS_MIN`] / [`DENSE_TYPES_MIN`]
/// counts plus [`DENSE_PER_SPECIES_MIN`], all checked on the finished scene.
const DENSE_PLAN: [(&str, usize, bool); 6] = [
    // (prototype, target, needs_water_adjacent)
    ("reed_cluster", 14, true),
    ("clustered_mushroom", 14, false),
    ("rosette_groundcover", 16, false),
    ("fan_frond", 12, false),
    ("funnel_mushroom", 16, false),
    ("parasol_mushroom", 12, false),
];

/// Every planned species must actually reach this many instances, so a scene
/// cannot pass the global count while one species is token-present.
pub const DENSE_PER_SPECIES_MIN: usize = 4;

fn water_within(tops: &Tops, x: i32, z: i32, radius: i32) -> bool {
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if let Some((_, m)) = tops.get(x + dx, z + dz) {
                if m == material::WATER {
                    return true;
                }
            }
        }
    }
    false
}

/// Dense 16 m representative tile: reuses the terrain tile plus the parasol
/// fixture and the five new flora prototypes. Places quota-driven,
/// habitat-clustered, fully foot-supported instances with varied yaw, leaving
/// the walkable corridor and the creek readable.
///
/// Fails explicitly (never a silent sparse scene) when the bounded search
/// cannot meet the instance, type or flora-cell thresholds for `seed`.
pub fn dense_tile(seed: u64) -> Result<DetailScene> {
    let tile = terrain_detail_tile("terrain_tile_16m", seed)?;
    let tops = Tops::build(&tile)?;
    let half = TILE_EDGE_CELLS / 2;
    let tile_cell = SCALE_TILE_M;

    // Prototype cache for footprint checks (built once, added to the scene after).
    let mut prototypes: Vec<DetailVolume> = Vec::new();
    prototypes.push(tile.clone());
    for id in FLORA_SPECIES {
        prototypes.push(flora_prototype(id)?);
    }
    let find = |id: &str| -> &DetailVolume {
        prototypes
            .iter()
            .find(|v| v.id() == id)
            .expect("built prototype")
    };

    // Dry, non-corridor candidate columns with deterministic hash order.
    let mut dry: Vec<(u64, i32, i32, i32)> = Vec::new();
    for x in -half..half {
        for z in -half..half {
            if x >= CORRIDOR_TILE_X.0 && x < CORRIDOR_TILE_X.1 {
                continue;
            }
            if let Some((h, m)) = tops.get(x, z) {
                if m == material::MOSS_TURF && h > TILE_WATER_LEVEL_CELLS {
                    dry.push((hash3(seed, x, z), x, z, h));
                }
            }
        }
    }
    if dry.is_empty() {
        return Err(DetailError::BudgetExceeded("dense tile has no dry footing"));
    }
    dry.sort();

    // Habitat cluster centres per species: three well-spread dry columns.
    let mut placed: Vec<([f32; 3], Yaw, &str)> = Vec::new();
    let mut per_species: std::collections::BTreeMap<&str, usize> = Default::default();
    for (species, quota, needs_water) in DENSE_PLAN {
        let radius = canopy_radius_m(species);
        // Cluster centres: hash-picked dry columns spread across the tile.
        let mut centres: Vec<(i32, i32)> = Vec::new();
        for (key, x, z, _) in dry.iter().copied() {
            let pick =
                (key ^ hash3(seed, species.len() as i32, centres.len() as i32)).is_multiple_of(7);
            if pick
                && centres
                    .iter()
                    .all(|(ox, oz)| (ox - x).abs() + (oz - z).abs() >= 16)
            {
                centres.push((x, z));
            }
            if centres.len() == 3 {
                break;
            }
        }
        if centres.len() < 3 {
            for (_, x, z, _) in dry.iter().step_by((dry.len() / 3).max(1)).take(3) {
                if !centres.contains(&(*x, *z)) {
                    centres.push((*x, *z));
                }
            }
        }
        // Candidates ordered by distance to the nearest own centre (clustering).
        let mut candidates: Vec<(i32, i32, i32)> = Vec::new();
        for (_, x, z, _) in dry.iter().copied() {
            if needs_water && !water_within(&tops, x, z, 3) {
                continue;
            }
            let nearest = centres
                .iter()
                .map(|(cx, cz)| (cx - x).abs() + (cz - z).abs())
                .min()
                .unwrap_or(0);
            candidates.push((nearest, x, z));
        }
        candidates.sort();
        let mut took = 0;
        for (_, x, z) in candidates {
            if took >= quota {
                break;
            }
            // Pairwise foot separation: boxes may touch canopies but not feet.
            let world_x = (x as f32 + 0.5) * tile_cell;
            let world_z = (z as f32 + 0.5) * tile_cell;
            let mut supported = None;
            for attempt in 0..4 {
                let candidate_yaw = DENSE_YAWS[((hash3(seed, x, z) % 4) as usize + attempt) % 4];
                let Some(h) = support_height(&tops, find(species), candidate_yaw, world_x, world_z)
                else {
                    continue;
                };
                // Reject a rooted-but-buried plant under the ACTUAL transform:
                // the feet may sit on moss while the body is driven into the
                // slope behind it. The search simply keeps looking, so this
                // costs candidates, never density targets.
                let candidate = Transform::new(
                    [world_x, (h + 1) as f32 * tile_cell, world_z],
                    candidate_yaw,
                )?;
                if intersects_terrain(&tile, find(species), &candidate) {
                    continue;
                }
                supported = Some((h, candidate_yaw));
                break;
            }
            let Some((support, yaw)) = supported else {
                continue;
            };
            let translation = [world_x, (support + 1) as f32 * tile_cell, world_z];
            let separated = placed.iter().all(|(other, _, other_species)| {
                let required = 0.4 + 0.35 * (radius + canopy_radius_m(other_species));
                (other[0] - translation[0]).abs() >= required
                    || (other[2] - translation[2]).abs() >= required
            });
            if !separated {
                continue;
            }
            placed.push((translation, yaw, species));
            took += 1;
        }
        if took < DENSE_PER_SPECIES_MIN {
            // A species that cannot be rooted at all on this terrain is a real
            // failure, not something to paper over with more of another plant.
            return Err(DetailError::BudgetExceeded(
                "dense tile placement search exhausted for a planned species",
            ));
        }
        per_species.insert(species, took);
    }

    let mut scene = DetailScene::new();
    for volume in prototypes {
        scene.add_prototype(volume)?;
    }
    scene.place("tile-0", "terrain_tile_16m", Transform::identity())?;
    let mut counters: std::collections::BTreeMap<&str, usize> = Default::default();
    for (index, (translation, yaw, species)) in placed.iter().enumerate() {
        let entry = counters.entry(species).or_insert(0);
        let instance = format!("{}-{entry}", species.trim_end_matches("_mushroom"));
        *entry += 1;
        scene.place(instance, species, Transform::new(*translation, *yaw)?)?;
        let _ = index;
    }

    // Thresholds are enforced on the authoritative scene counts, never assumed.
    let counts = scene.counts();
    let vegetation = counts.instances - 1;
    let tile_cells = scene
        .prototype("terrain_tile_16m")
        .expect("tile prototype")
        .occupied_cells();
    let flora_cells = counts.expanded_occupied_cells - tile_cells;
    let mut types = BTreeSet::new();
    for draw in scene.draws() {
        if draw.prototype != "terrain_tile_16m" {
            types.insert(draw.prototype);
        }
    }
    if vegetation < DENSE_VEGETATION_MIN
        || flora_cells < DENSE_FLORA_CELLS_MIN
        || types.len() < DENSE_TYPES_MIN
    {
        return Err(DetailError::BudgetExceeded(
            "dense tile below density thresholds",
        ));
    }
    Ok(scene)
}

/// True when every material in the volume has the expected collision policy.
pub fn assert_policy(volume: &DetailVolume, policy: MaterialPolicy) -> bool {
    volume
        .iter_cells()
        .all(|(_, m)| material_policy(m) == policy)
}
