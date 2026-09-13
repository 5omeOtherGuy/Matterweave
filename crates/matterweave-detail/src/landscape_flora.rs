//! Landscape flora catalogue: cheap voxel prototypes for the dense fields.
//!
//! This is the geometry half of the dense-field work. It turns the placement
//! vocabulary of [`matterweave_core::landscape`] ([`FloraKind`], [`FloraSite`])
//! into buildable [`DetailVolume`] prototypes: grass tufts, three flower
//! colours, ferns, shrubs, cacti, broadleaf and conifer trees. A later worker
//! instances them in the landscape sample and animates them.
//!
//! Conventions follow [`crate::flora`] exactly: integer construction,
//! connected bodies (one six-connected solid, not floating cells), a compact
//! ground-contact pad at local y = 0, deterministic id-independent geometry,
//! and an explicit per-prototype scale. Everything here builds at 12.5 cm
//! cells ([`LANDSCAPE_LEAF_SCALE_M`]), including trees: at that scale the
//! large broadleaf is ~3.5 m tall, which fits the caps below with room to
//! spare, so no 25 cm exception is needed.
//!
//! Every prototype is fully [`MaterialPolicy::Decorative`]: nothing here
//! collides. Walking through a trunk is a documented choice for this version,
//! not an accident; a later worker may promote trunks to collidable wood.
//!
//! `reed_cluster` and `rosette_groundcover` already exist in [`crate::flora`]
//! and are reused for swamp and forest ground rather than duplicated here.

use crate::{
    flora::{reed_cluster, rosette_groundcover},
    material, DetailError, DetailVolume, MaterialPolicy, Result, Scale,
};
use matterweave_core::landscape::{FloraKind, FloraSite};

/// Landscape flora generator version, recorded in gallery manifests.
pub const LANDSCAPE_FLORA_VERSION: u32 = 1;
/// Explicit scale for every prototype in this module (12.5 cm cells).
pub const LANDSCAPE_LEAF_SCALE_M: f32 = 0.125;

/// Maximum occupied cells per prototype class (enforced by test, not by hope).
pub const GRASS_CELL_CAP: usize = 40;
pub const FLOWER_CELL_CAP: usize = 32;
pub const FERN_CELL_CAP: usize = 96;
pub const SHRUB_CELL_CAP: usize = 384;
pub const CACTUS_CELL_CAP: usize = 256;
pub const TREE_CELL_CAP: usize = 4096;
/// Maximum meshed triangles at [`crate::Lod::Source`] per prototype class.
pub const GRASS_TRI_CAP: usize = 120;
pub const FLOWER_TRI_CAP: usize = 90;
pub const FERN_TRI_CAP: usize = 240;
pub const SHRUB_TRI_CAP: usize = 700;
pub const CACTUS_TRI_CAP: usize = 500;
pub const TREE_TRI_CAP: usize = 4000;

/// Structural per-16-m-square placement budget: one 16 m square holds 8x8
/// [`matterweave_core::landscape::FLORA_CELL_M`] flora cells with at most 8
/// sites each (4 metre-columns x grass-slot + flower-slot) plus 2x2
/// [`matterweave_core::landscape::TREE_CELL_M`] tree cells with at most one
/// tree each: 64 * 8 + 4 = 516.
pub const MAX_SITES_PER_16M_SQUARE: usize = 516;

/// Every new prototype id built by this module.
pub const LANDSCAPE_FLORA_SPECIES: &[&str] = &[
    "grass_tuft_s",
    "grass_tuft_m",
    "grass_tuft_l",
    "flower_red_s",
    "flower_red_m",
    "flower_red_l",
    "flower_white_s",
    "flower_white_m",
    "flower_white_l",
    "flower_yellow_s",
    "flower_yellow_m",
    "flower_yellow_l",
    "fern",
    "shrub",
    "cactus",
    "tree_broadleaf_s",
    "tree_broadleaf_m",
    "tree_broadleaf_l",
    "tree_conifer_s",
    "tree_conifer_m",
    "tree_conifer_l",
];

/// Walk from `from` to `to` one axis at a time (x, then z, then y), filling
/// every intermediate air cell, so a rasterised stem or blade is six-connected
/// instead of a chain of diagonal touches. Occupied cells are left alone.
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

/// True when the volume is a single six-connected body.
pub fn is_single_body(volume: &DetailVolume) -> bool {
    use std::collections::BTreeSet;
    let cells: BTreeSet<[i32; 3]> = volume.iter_cells().map(|(c, _)| c).collect();
    let Some(start) = cells.iter().next().copied() else {
        return false;
    };
    let mut seen: BTreeSet<[i32; 3]> = BTreeSet::new();
    let mut stack = vec![start];
    seen.insert(start);
    while let Some(c) = stack.pop() {
        for step in [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1],
        ] {
            let n = [c[0] + step[0], c[1] + step[1], c[2] + step[2]];
            if cells.contains(&n) && seen.insert(n) {
                stack.push(n);
            }
        }
    }
    seen.len() == cells.len()
}

/// Compact plus-shaped ground-contact pad at y = 0.
fn contact_pad(volume: &mut DetailVolume, m: u8) -> Result<()> {
    for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
        volume.set([dx, 0, dz], m)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Grass tufts
// ---------------------------------------------------------------------------

/// Blade specs: (base [x, z], height in blade cells, lean [x, z]). All sizes
/// carry four blades on the plus-pad; the size variant scales heights
/// (0.75x/1.0x/1.25x), not the blade count, which keeps the large tuft inside
/// its 40-cell cap (lean elbows cost extra cells on tall blades).
const GRASS_BLADES_S: [([i32; 2], i32, [i32; 2]); 4] = [
    ([0, 0], 3, [1, 0]),
    ([1, 0], 4, [0, 1]),
    ([0, 1], 3, [-1, 0]),
    ([-1, 0], 4, [0, -1]),
];
const GRASS_BLADES_M: [([i32; 2], i32, [i32; 2]); 4] = [
    ([0, 0], 4, [1, 0]),
    ([1, 0], 5, [0, 1]),
    ([0, 1], 4, [-1, 0]),
    ([-1, 0], 5, [0, -1]),
];
const GRASS_BLADES_L: [([i32; 2], i32, [i32; 2]); 4] = [
    ([0, 0], 5, [1, 0]),
    ([1, 0], 6, [0, 1]),
    ([0, 1], 5, [-1, 0]),
    ([-1, 0], 6, [0, -1]),
];

/// One blade: a leaning stalk of `height` cells grown from the pad with a
/// lighter tip cell. Sideways steps happen at most every third row and are
/// routed through [`set_path`], so the blade is one connected stalk.
fn grass_blade(
    volume: &mut DetailVolume,
    base: [i32; 2],
    height: i32,
    lean: [i32; 2],
) -> Result<()> {
    let mut prev = [base[0], 1, base[1]];
    volume.set(prev, material::GRASS_BLADE)?;
    for level in 1..height {
        let mut next = [prev[0], prev[1] + 1, prev[2]];
        if level % 3 == 0 {
            if lean[0] != 0 {
                next[0] += lean[0];
            } else {
                next[2] += lean[1];
            }
        }
        set_path(volume, prev, next, material::GRASS_BLADE)?;
        prev = next;
    }
    let tip = [prev[0], prev[1] + 1, prev[2]];
    set_path(volume, prev, tip, material::GRASS_TIP)?;
    Ok(())
}

fn grass_tuft_with(id: &str, blades: &[([i32; 2], i32, [i32; 2])]) -> Result<DetailVolume> {
    let scale = Scale::new(LANDSCAPE_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    contact_pad(&mut volume, material::GRASS_BLADE)?;
    for (base, height, lean) in blades.iter().copied() {
        grass_blade(&mut volume, base, height, lean)?;
    }
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

pub fn grass_tuft_s(id: &str) -> Result<DetailVolume> {
    grass_tuft_with(id, &GRASS_BLADES_S)
}
pub fn grass_tuft_m(id: &str) -> Result<DetailVolume> {
    grass_tuft_with(id, &GRASS_BLADES_M)
}
pub fn grass_tuft_l(id: &str) -> Result<DetailVolume> {
    grass_tuft_with(id, &GRASS_BLADES_L)
}

// ---------------------------------------------------------------------------
// Flowers
// ---------------------------------------------------------------------------

/// One flower: a single-cell pad, a green stem of `stem_h` cells (always
/// 2..=4), two leaf cells flanking the stem at `leaf_y`, four petal arms in
/// a cross (plus a fifth arm when `full_bloom`), and one heart cell as the
/// cross centre. Leaves at the top stem cell read as sepals under the bloom
/// and share faces with both stem and petals, which keeps the meshed triangle
/// count inside the 90-triangle cap.
///
/// Stem heights differ per colour (red 4, white 3, yellow 2 at medium) so a
/// meadow mixes silhouettes; the size variants shift the stem by one within
/// 2..=4 and differentiate further by bloom fullness and leaf height, since
/// strictly increasing stems, distinct mediums and the 2..=4 bound cannot
/// all hold at once (only three integer heights exist).
fn flower_with(
    id: &str,
    petal: u8,
    stem_h: i32,
    full_bloom: bool,
    leaf_y: i32,
) -> Result<DetailVolume> {
    let scale = Scale::new(LANDSCAPE_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    volume.set([0, 0, 0], material::GRASS_BLADE)?;
    for y in 1..=stem_h {
        volume.set([0, y, 0], material::GRASS_BLADE)?;
    }
    volume.set([1, leaf_y, 0], material::GRASS_BLADE)?;
    volume.set([-1, leaf_y, 0], material::GRASS_BLADE)?;
    // Cross arms; the fifth arm (-x) only on full blooms.
    let mut arms = vec![(1, 0), (0, 1), (0, -1)];
    if full_bloom {
        arms.push((-1, 0));
    }
    for (dx, dz) in arms {
        volume.set([dx, stem_h + 1, dz], petal)?;
    }
    volume.set([0, stem_h + 1, 0], material::FLOWER_HEART)?;
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

pub fn flower_red_s(id: &str) -> Result<DetailVolume> {
    flower_with(id, material::FLOWER_PETAL_RED, 3, false, 3)
}
pub fn flower_red_m(id: &str) -> Result<DetailVolume> {
    flower_with(id, material::FLOWER_PETAL_RED, 4, false, 4)
}
pub fn flower_red_l(id: &str) -> Result<DetailVolume> {
    flower_with(id, material::FLOWER_PETAL_RED, 4, true, 4)
}
pub fn flower_white_s(id: &str) -> Result<DetailVolume> {
    flower_with(id, material::FLOWER_PETAL_WHITE, 2, false, 2)
}
pub fn flower_white_m(id: &str) -> Result<DetailVolume> {
    flower_with(id, material::FLOWER_PETAL_WHITE, 3, false, 3)
}
pub fn flower_white_l(id: &str) -> Result<DetailVolume> {
    flower_with(id, material::FLOWER_PETAL_WHITE, 4, true, 4)
}
pub fn flower_yellow_s(id: &str) -> Result<DetailVolume> {
    flower_with(id, material::FLOWER_PETAL_YELLOW, 2, false, 1)
}
pub fn flower_yellow_m(id: &str) -> Result<DetailVolume> {
    flower_with(id, material::FLOWER_PETAL_YELLOW, 2, false, 2)
}
pub fn flower_yellow_l(id: &str) -> Result<DetailVolume> {
    flower_with(id, material::FLOWER_PETAL_YELLOW, 3, false, 3)
}

// ---------------------------------------------------------------------------
// Fern
// ---------------------------------------------------------------------------

/// Axis frond lengths (6..7 cells of reach); the two diagonal fronds are
/// short staircases of reach 5. Diagonal reach costs two cells per unit (a
/// staircase step moves one axis at a time to stay six-connected), so long
/// diagonals would blow the 96-cell cap.
const FERN_FRONDS: [([i32; 2], i32); 4] = [([1, 0], 6), ([0, 1], 7), ([-1, 0], 6), ([0, -1], 7)];
const FERN_DIAGONALS: [([i32; 2], i32); 2] = [([1, 1], 4), ([-1, -1], 4)];

/// Fern: six arched fronds radiating from a small pad (four axis fronds
/// plus two short diagonal staircases). Each frond leaves the crown at y = 1,
/// runs one cell higher, and settles back to a ground-touching tip; the arch
/// uses exactly two height steps so the flat runs merge into long greedy
/// quads (this is what keeps the meshed triangle count inside the 240 cap).
/// One midrib row per frond carries a single flanking cell, making that row
/// two cells thick. Flat runs, single steps and flank cells are all routed
/// through [`set_path`], so every frond is one connected arch.
pub fn fern(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(LANDSCAPE_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    contact_pad(&mut volume, material::SHRUB_STEM)?;
    // Raised crown: fronds leave at y = 2 and settle to ground-touching tips,
    // so each frond arches with a single height step (touchdown), which keeps
    // the flat runs mergeable into long greedy quads.
    volume.set([0, 1, 0], material::SHRUB_STEM)?;
    volume.set([0, 2, 0], material::SHRUB_STEM)?;
    for (index, (dir, length)) in FERN_FRONDS.into_iter().enumerate() {
        let mut prev = [0, 2, 0];
        if volume.get(prev) == material::AIR {
            volume.set(prev, material::SHRUB_STEM)?;
        }
        for t in 1..=length {
            // Flat run at crown height with a single touchdown step at the tip.
            let y = if t == length { 1 } else { 2 };
            let cell = [dir[0] * t, y, dir[1] * t];
            set_path(&mut volume, prev, cell, material::SHRUB_LEAF)?;
            prev = cell;
        }
        // Single flanking cell on one midrib row (alternating sides): that
        // row is two cells thick. Its in-line faces merge with the run's.
        let mid = length / 2;
        let perp = [-dir[1], dir[0]];
        let side = if index % 2 == 0 { 1 } else { -1 };
        let flank = [
            dir[0] * mid + perp[0] * side,
            2,
            dir[1] * mid + perp[1] * side,
        ];
        set_path(
            &mut volume,
            [dir[0] * mid, 2, dir[1] * mid],
            flank,
            material::SHRUB_LEAF,
        )?;
    }
    // Diagonal staircases: alternate x/z steps so every step moves one axis
    // (six-connected by construction), arched like the axis fronds (up after
    // the sheath, touchdown tip) with one flanking cell at mid-length.
    for (dir, reach) in FERN_DIAGONALS {
        let mut prev = [0, 2, 0];
        // Staircase order: x-step, z-step per unit, so `s` maps to a path.
        let mut k = [0, 0];
        for s in 1..=(2 * reach) {
            k[(s % 2) as usize] += 1;
            let y = if s >= 2 * reach - 1 { 1 } else { 2 };
            let cell = [dir[0] * k[0], y, dir[1] * k[1]];
            // The arch lift can coincide with a horizontal step; route through
            // set_path so the staircase never splits.
            set_path(&mut volume, prev, cell, material::SHRUB_LEAF)?;
            prev = cell;
        }
        let mid = [dir[0] * (reach / 2 + 1), 2, dir[1] * (reach / 2 + 1)];
        let side = [mid[0] - dir[1], mid[1], mid[2] + dir[0]];
        set_path(&mut volume, mid, side, material::SHRUB_LEAF)?;
    }
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

// ---------------------------------------------------------------------------
// Shrub
// ---------------------------------------------------------------------------

/// Fill a solid ellipsoid leaf mass around `centre`: cells with `d <= 1.0`
/// become leaf. Solid rather than hollow: the pole cells of a hollow shell
/// have no six-connected neighbour (every neighbour falls in the carved
/// interior or outside), which splits the prototype into several bodies, and
/// a solid mass meshes only its outer surface, so it costs fewer triangles.
/// The seed cell is always inside the radii, so a crown grown over a branch
/// tip merges with the wood.
fn leaf_shell(volume: &mut DetailVolume, centre: [i32; 3], radii: [i32; 3], mat: u8) -> Result<()> {
    let [cx, cy, cz] = centre;
    let [rx, ry, rz] = radii;
    for x in -rx..=rx {
        for y in -ry..=ry {
            for z in -rz..=rz {
                let d = (x * x) as f32 / (rx * rx) as f32
                    + (y * y) as f32 / (ry * ry) as f32
                    + (z * z) as f32 / (rz * rz) as f32;
                if d > 1.0 {
                    continue;
                }
                let cell = [cx + x, cy + y, cz + z];
                if cell[1] < 0 {
                    continue;
                }
                if volume.get(cell) == material::AIR {
                    volume.set(cell, mat)?;
                }
            }
        }
    }
    Ok(())
}

/// Shrub: a rounded low bush, 9 wide and 7 tall, as a dense solid leaf mass
/// over a short stem. The stem top reaches into the leaf, joining wood to leaf.
pub fn shrub(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(LANDSCAPE_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    for x in -1..=1 {
        for z in -1..=1 {
            volume.set([x, 0, z], material::SHRUB_STEM)?;
        }
    }
    for y in 1..=4 {
        for x in -1..=1 {
            for z in -1..=1 {
                if x * x + z * z <= 1 {
                    volume.set([x, y, z], material::SHRUB_STEM)?;
                }
            }
        }
    }
    leaf_shell(&mut volume, [0, 6, 0], [4, 3, 4], material::SHRUB_LEAF)?;
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

// ---------------------------------------------------------------------------
// Cactus
// ---------------------------------------------------------------------------

/// Cactus: a plus-cross-section column 8 cells tall with two arms and pale
/// spine cells on the silhouette. Arms are walked from the column so they read
/// as limbs, not stuck-on boxes.
pub fn cactus(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(LANDSCAPE_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    contact_pad(&mut volume, material::CACTUS_BODY)?;
    for y in 1..=7 {
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            volume.set([dx, y, dz], material::CACTUS_BODY)?;
        }
    }
    volume.set([0, 8, 0], material::CACTUS_BODY)?;
    // Arms: out from the column, then up.
    set_path(&mut volume, [1, 3, 0], [3, 3, 0], material::CACTUS_BODY)?;
    set_path(&mut volume, [3, 3, 0], [3, 5, 0], material::CACTUS_BODY)?;
    set_path(&mut volume, [-1, 5, 0], [-3, 5, 0], material::CACTUS_BODY)?;
    set_path(&mut volume, [-3, 5, 0], [-3, 7, 0], material::CACTUS_BODY)?;
    // Spines on the silhouette: every other row, four sides, plus arm tips.
    for y in (2..=7).step_by(2) {
        for (dx, dz) in [(2, 0), (-2, 0), (0, 2), (0, -2)] {
            if volume.get([dx, y, dz]) == material::AIR {
                volume.set([dx, y, dz], material::CACTUS_SPINE)?;
            }
        }
    }
    for tip in [[3, 6, 0], [-3, 8, 0], [0, 9, 0]] {
        if volume.get(tip) == material::AIR {
            volume.set(tip, material::CACTUS_SPINE)?;
        }
    }
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

// ---------------------------------------------------------------------------
// Trees
// ---------------------------------------------------------------------------

/// Broadleaf branch waypoints: each arm is walked from inside the 2x2 trunk to
/// a tip inside the canopy shell (d < 1), so limbs merge with leaf.
const BROADLEAF_BRANCHES_S: [[i32; 3]; 8] = [
    [0, 6, 0],
    [2, 8, 0],
    [0, 7, 0],
    [-2, 8, 0],
    [1, 6, 1],
    [1, 8, 2],
    [0, 7, 0],
    [0, 8, -1],
];
const BROADLEAF_BRANCHES_M: [[i32; 3]; 8] = [
    [1, 9, 0],
    [2, 12, 0],
    [0, 10, 0],
    [-2, 12, 0],
    [1, 9, 1],
    [0, 12, 1],
    [0, 10, 0],
    [0, 12, -1],
];
const BROADLEAF_BRANCHES_L: [[i32; 3]; 8] = [
    [1, 12, 0],
    [3, 15, 0],
    [0, 13, 0],
    [-3, 15, 0],
    [1, 12, 1],
    [1, 15, 3],
    [0, 13, 0],
    [0, 15, -2],
];

/// Broadleaf tree: 2x2 trunk, four walked limbs and a rounded solid canopy
/// of `radii` centred at `canopy_y`. The trunk runs up into the canopy,
/// joining bark to leaf.
fn broadleaf_with(
    id: &str,
    canopy_y: i32,
    radii: [i32; 3],
    arms: &[[i32; 3]],
) -> Result<DetailVolume> {
    let scale = Scale::new(LANDSCAPE_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    for x in -1..=1 {
        for z in -1..=1 {
            volume.set([x, 0, z], material::TREE_BARK)?;
        }
    }
    for y in 1..=canopy_y {
        for x in 0..=1 {
            for z in 0..=1 {
                volume.set([x, y, z], material::TREE_BARK)?;
            }
        }
    }
    for arm in arms.chunks(2) {
        set_path(&mut volume, arm[0], arm[1], material::TREE_BARK)?;
    }
    leaf_shell(&mut volume, [0, canopy_y, 0], radii, material::TREE_LEAF)?;
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

pub fn tree_broadleaf_s(id: &str) -> Result<DetailVolume> {
    broadleaf_with(id, 9, [3, 2, 3], &BROADLEAF_BRANCHES_S)
}
pub fn tree_broadleaf_m(id: &str) -> Result<DetailVolume> {
    broadleaf_with(id, 14, [4, 3, 4], &BROADLEAF_BRANCHES_M)
}
pub fn tree_broadleaf_l(id: &str) -> Result<DetailVolume> {
    broadleaf_with(id, 18, [5, 4, 5], &BROADLEAF_BRANCHES_L)
}

/// Conifer needle rings: (height, radius), shrinking towards the top.
const CONIFER_RINGS_S: [(i32, i32); 4] = [(5, 3), (7, 2), (9, 2), (11, 1)];
const CONIFER_RINGS_M: [(i32, i32); 5] = [(7, 4), (10, 3), (12, 3), (14, 2), (16, 1)];
const CONIFER_RINGS_L: [(i32, i32); 6] = [(9, 5), (12, 4), (15, 3), (18, 3), (20, 2), (22, 1)];

/// Conifer: a straight 2x2 trunk with stacked full-disc needle rings that
/// shrink towards the top and a needle tip. Rings are laid over air only, so
/// the trunk shows through and every ring merges with it.
fn conifer_with(id: &str, trunk_h: i32, rings: &[(i32, i32)]) -> Result<DetailVolume> {
    let scale = Scale::new(LANDSCAPE_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    for x in -1..=1 {
        for z in -1..=1 {
            volume.set([x, 0, z], material::TREE_BARK)?;
        }
    }
    for y in 1..=trunk_h {
        for x in 0..=1 {
            for z in 0..=1 {
                volume.set([x, y, z], material::TREE_BARK)?;
            }
        }
    }
    for (y, radius) in rings.iter().copied() {
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x * x + z * z <= radius * radius {
                    let cell = [x, y, z];
                    if volume.get(cell) == material::AIR {
                        volume.set(cell, material::TREE_NEEDLE)?;
                    }
                }
            }
        }
    }
    let tip = [0, trunk_h + 1, 0];
    if volume.get(tip) == material::AIR {
        volume.set(tip, material::TREE_NEEDLE)?;
    }
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

pub fn tree_conifer_s(id: &str) -> Result<DetailVolume> {
    conifer_with(id, 12, &CONIFER_RINGS_S)
}
pub fn tree_conifer_m(id: &str) -> Result<DetailVolume> {
    conifer_with(id, 18, &CONIFER_RINGS_M)
}
pub fn tree_conifer_l(id: &str) -> Result<DetailVolume> {
    conifer_with(id, 24, &CONIFER_RINGS_L)
}

/// Build one landscape flora prototype by id. The reuse ids `reed_cluster`
/// and `rosette_groundcover` delegate to the existing [`crate::flora`]
/// catalogue; they are not duplicated here.
pub fn landscape_prototype(id: &str) -> Result<DetailVolume> {
    match id {
        "grass_tuft_s" => grass_tuft_s(id),
        "grass_tuft_m" => grass_tuft_m(id),
        "grass_tuft_l" => grass_tuft_l(id),
        "flower_red_s" => flower_red_s(id),
        "flower_red_m" => flower_red_m(id),
        "flower_red_l" => flower_red_l(id),
        "flower_white_s" => flower_white_s(id),
        "flower_white_m" => flower_white_m(id),
        "flower_white_l" => flower_white_l(id),
        "flower_yellow_s" => flower_yellow_s(id),
        "flower_yellow_m" => flower_yellow_m(id),
        "flower_yellow_l" => flower_yellow_l(id),
        "fern" => fern(id),
        "shrub" => shrub(id),
        "cactus" => cactus(id),
        "tree_broadleaf_s" => tree_broadleaf_s(id),
        "tree_broadleaf_m" => tree_broadleaf_m(id),
        "tree_broadleaf_l" => tree_broadleaf_l(id),
        "tree_conifer_s" => tree_conifer_s(id),
        "tree_conifer_m" => tree_conifer_m(id),
        "tree_conifer_l" => tree_conifer_l(id),
        "reed_cluster" => reed_cluster(id),
        "rosette_groundcover" => rosette_groundcover(id),
        _ => Err(DetailError::UnknownPrototype(id.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Placement bridge
// ---------------------------------------------------------------------------

/// Small/medium/large variant from a site's scale: under 6/8 is small,
/// under 10/8 is medium, the rest is large.
fn variant(scale_eighths: u8) -> &'static str {
    if scale_eighths < 6 {
        "s"
    } else if scale_eighths < 10 {
        "m"
    } else {
        "l"
    }
}

/// Concrete prototype id for a landscape flora site. Total over [`FloraKind`]:
/// every kind maps to a prototype that builds, never a panic. Sized kinds use
/// the `_s|_m|_l` variant from `scale_eighths`; `Reed` reuses the existing
/// `reed_cluster` prototype.
pub fn prototype_for(site: &FloraSite) -> &'static str {
    match (site.kind, variant(site.scale_eighths)) {
        (FloraKind::GrassTuft, "s") => "grass_tuft_s",
        (FloraKind::GrassTuft, "l") => "grass_tuft_l",
        (FloraKind::GrassTuft, _) => "grass_tuft_m",
        (FloraKind::FlowerRed, "s") => "flower_red_s",
        (FloraKind::FlowerRed, "l") => "flower_red_l",
        (FloraKind::FlowerRed, _) => "flower_red_m",
        (FloraKind::FlowerWhite, "s") => "flower_white_s",
        (FloraKind::FlowerWhite, "l") => "flower_white_l",
        (FloraKind::FlowerWhite, _) => "flower_white_m",
        (FloraKind::FlowerYellow, "s") => "flower_yellow_s",
        (FloraKind::FlowerYellow, "l") => "flower_yellow_l",
        (FloraKind::FlowerYellow, _) => "flower_yellow_m",
        (FloraKind::Fern, _) => "fern",
        (FloraKind::Shrub, _) => "shrub",
        (FloraKind::TreeBroadleaf, "s") => "tree_broadleaf_s",
        (FloraKind::TreeBroadleaf, "l") => "tree_broadleaf_l",
        (FloraKind::TreeBroadleaf, _) => "tree_broadleaf_m",
        (FloraKind::TreeConifer, "s") => "tree_conifer_s",
        (FloraKind::TreeConifer, "l") => "tree_conifer_l",
        (FloraKind::TreeConifer, _) => "tree_conifer_m",
        (FloraKind::Reed, _) => "reed_cluster",
        (FloraKind::Cactus, _) => "cactus",
    }
}

/// Broad gallery class for a landscape prototype id.
pub fn landscape_flora_class(id: &str) -> &'static str {
    if id.starts_with("grass_tuft") || id == "fern" || id == "reed_cluster" {
        "flora-grass"
    } else if id.starts_with("flower_") || id == "rosette_groundcover" {
        "flora-flower"
    } else if id.starts_with("tree_") {
        "flora-tree"
    } else if id == "shrub" {
        "flora-shrub"
    } else if id == "cactus" {
        "flora-cactus"
    } else {
        "flora-unknown"
    }
}

/// True when every material in the volume has the expected collision policy.
pub fn assert_landscape_policy(volume: &DetailVolume) -> bool {
    volume
        .iter_cells()
        .all(|(_, m)| crate::material_policy(m) == MaterialPolicy::Decorative)
}
