//! Landscape flora catalogue: cheap voxel prototypes for the dense fields.
//!
//! This is the geometry half of the dense-field work. It turns the placement
//! vocabulary of [`matterweave_core::landscape`] ([`FloraKind`], [`FloraSite`])
//! into buildable [`DetailVolume`] prototypes: grass clumps, three flower
//! colours, ferns, shrubs, cacti, broadleaf and conifer trees. The landscape
//! sample pools these prototypes, instances them along the plan and animates
//! them through the renderer's wind path.
//!
//! Conventions follow [`crate::flora`] exactly: integer construction,
//! connected bodies (one six-connected solid, not floating cells), a compact
//! ground-contact pad at local y = 0, deterministic id-independent geometry,
//! and an explicit per-prototype scale. Ground cover is built at 6.25 cm cells
//! ([`LANDSCAPE_FINE_FLORA_SCALE_M`]) so a blade is one voxel across;
//! everything woody builds at 12.5 cm ([`LANDSCAPE_LEAF_SCALE_M`]), where the
//! large broadleaf is ~6.9 m tall and fits the caps below with room to spare.
//!
//! Every prototype is fully [`MaterialPolicy::Decorative`]: nothing here
//! collides. Walking through a trunk is a documented choice for this version,
//! not an accident; a later worker may promote trunks to collidable wood.
//!
//! `reed_cluster` and `rosette_groundcover` already exist in [`crate::flora`]
//! and are reused for swamp and forest ground rather than duplicated here.

use crate::{
    flora::{reed_cluster, rosette_groundcover},
    material, DetailError, DetailVolume, MaterialPolicy, Result, Scale, SCALE_FINE_M,
};
use matterweave_core::landscape::{FloraKind, FloraSite};

/// Landscape flora generator version, recorded in gallery manifests.
///
/// Version 3: ground cover is a clump of thin fine-scale blades (6.25 cm cells,
/// 8..=20 cells tall) instead of 12.5 cm cube-like tufts, flowers are thin
/// fine-scale stems with a bloom, and both tree kinds carry a structured crown
/// instead of a solid leaf ellipsoid.
///
/// Version 2: grass tufts are eight-blade leaning fans in a 3x3 clump with a
/// two-cell tip (version 1 was four straight posts with a one-cell tip).
pub const LANDSCAPE_FLORA_VERSION: u32 = 3;
/// Explicit scale for the woody prototypes in this module (12.5 cm cells):
/// trees, ferns, shrubs and cacti. Ground cover is built at
/// [`LANDSCAPE_FINE_FLORA_SCALE_M`] instead.
pub const LANDSCAPE_LEAF_SCALE_M: f32 = 0.125;
/// Scale of the ground-cover prototypes (grass clumps and flowers): 6.25 cm
/// cells, [`SCALE_FINE_M`]. A blade is one cell across, so the cell size is the
/// blade width: half the woody scale is what makes a blade read as a blade
/// rather than as a post, and 8..=20 cells of height is 0.5..=1.25 m.
pub const LANDSCAPE_FINE_FLORA_SCALE_M: f32 = SCALE_FINE_M;

/// Maximum occupied cells per prototype class (enforced by test, not by hope).
/// A ground-cover prototype carries one cell per blade level plus its contact
/// pad, so the fine-scale clumps are cell-heavy and triangle-cheap: the greedy
/// mesher merges a 1x1 stalk into four long side quads.
pub const GRASS_CELL_CAP: usize = 512;
pub const FLOWER_CELL_CAP: usize = 192;
pub const FERN_CELL_CAP: usize = 96;
pub const SHRUB_CELL_CAP: usize = 384;
pub const CACTUS_CELL_CAP: usize = 256;
pub const TREE_CELL_CAP: usize = 4096;
/// Maximum meshed triangles at [`crate::Lod::Source`] per prototype class.
pub const GRASS_TRI_CAP: usize = 512;
pub const FLOWER_TRI_CAP: usize = 384;
pub const FERN_TRI_CAP: usize = 240;
pub const SHRUB_TRI_CAP: usize = 700;
pub const CACTUS_TRI_CAP: usize = 500;
pub const TREE_TRI_CAP: usize = 6000;

/// Structural per-16-m-square placement budget: one 16 m square holds 8x8
/// [`matterweave_core::landscape::FLORA_CELL_M`] flora cells with at most 8
/// sites each (4 metre-columns x grass slot + understory slot + flower slot,
/// the cell's [`matterweave_core::landscape::MAX_FLORA_PER_CELL`]) plus 2x2
/// [`matterweave_core::landscape::TREE_CELL_M`] tree cells with at most one
/// tree each: 64 * 12 + 4 = 772.
pub const MAX_SITES_PER_16M_SQUARE: usize = 772;

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
// Grass clumps
// ---------------------------------------------------------------------------

/// Blades in the medium grass clump. The tier tables below carry the other two
/// counts: the small clump has one blade per cell of a 3x3 pad (9) and the
/// large clump one per cell of its 21-cell pad. A clump is a *cluster of thin
/// vertical elements*, so the blade count is the pad cell count: every blade
/// rises from its own cell, one fine voxel across.
pub const GRASS_BLADES_PER_TUFT: usize = 13;
/// Minimum cells of each blade that carry the lighter [`material::GRASS_TIP`].
/// A taller blade carries more (a third of its height, at least this many), so
/// the tip colour is a constant fraction of the silhouette rather than a fixed
/// cap on tall blades.
pub const GRASS_TIP_CELLS_PER_BLADE: usize = 2;
/// Shortest blade of any tier, in fine voxels. The brief's floor: 8 cells of
/// 6.25 cm is a 50 cm blade.
pub const GRASS_MIN_HEIGHT_CELLS: i32 = 8;
/// Tallest blade of any tier, in fine voxels: 20 cells is 1.25 m.
pub const GRASS_MAX_HEIGHT_CELLS: i32 = 20;

/// One tier of the grass catalogue: the blade bases on the ground pad (local
/// x/z, all at y = 0), the extra height each base's blade carries above
/// `base_height`, and the cells above which that blade leans outward.
struct GrassTier {
    /// Blade bases. The pad is exactly this set of cells at y = 0, so a blade
    /// always has its own rooted cell and the clump is one connected body.
    bases: &'static [[i32; 2]],
    /// Height in fine voxels of the blade at each base, in base order. Every
    /// entry is within [`GRASS_MIN_HEIGHT_CELLS`]..=[`GRASS_MAX_HEIGHT_CELLS`]
    /// (the test enforces it), and no two adjacent entries are equal, so the
    /// clump silhouette is ragged rather than a mown hedge.
    heights: &'static [i32],
    /// Cells above the base at which each blade's single lean starts, in base
    /// order; `0` leaves that blade vertical. A lean is one cell outward, so a
    /// blade is a straight stalk with at most one elbow.
    leans: &'static [i32],
}

/// Small clump: five blades on the corners and centre of a 3x3 foot, 8..=12
/// cells. The bases are two cells apart so no two blades touch at the root.
const GRASS_BASES_S: [[i32; 2]; 5] = [[0, 0], [1, 1], [-1, -1], [1, -1], [-1, 1]];
const GRASS_HEIGHTS_S: [i32; 5] = [13, 10, 12, 8, 11];
const GRASS_LEANS_S: [i32; 5] = [4, 2, 4, 2, 3];
/// Medium clump: nine blades on an even 5x5 lattice, 10..=16 cells.
const GRASS_BASES_M: [[i32; 2]; 9] = [
    [0, 0],
    [2, 0],
    [-2, 0],
    [0, 2],
    [0, -2],
    [2, 2],
    [-2, -2],
    [2, -2],
    [-2, 2],
];
const GRASS_HEIGHTS_M: [i32; 9] = [17, 13, 15, 12, 16, 14, 11, 14, 13];
const GRASS_LEANS_M: [i32; 9] = [5, 2, 4, 2, 4, 3, 2, 4, 3];
/// Large clump: twelve blades on an even 7x7 lattice (its four corners are
/// dropped to keep the meshed cost down), 13..=20 cells - the tall, wide clump a
/// dense field needs.
const GRASS_BASES_L: [[i32; 2]; 12] = [
    [3, 1],
    [3, -1],
    [1, 3],
    [1, 1],
    [1, -1],
    [1, -3],
    [-1, 3],
    [-1, 1],
    [-1, -1],
    [-1, -3],
    [-3, 1],
    [-3, -1],
];
const GRASS_HEIGHTS_L: [i32; 12] = [20, 16, 18, 15, 20, 17, 14, 17, 19, 16, 18, 15];
const GRASS_LEANS_L: [i32; 12] = [7, 4, 5, 3, 6, 4, 2, 5, 6, 4, 7, 5];

const GRASS_TIERS: [GrassTier; 3] = [
    GrassTier {
        bases: &GRASS_BASES_S,
        heights: &GRASS_HEIGHTS_S,
        leans: &GRASS_LEANS_S,
    },
    GrassTier {
        bases: &GRASS_BASES_M,
        heights: &GRASS_HEIGHTS_M,
        leans: &GRASS_LEANS_M,
    },
    GrassTier {
        bases: &GRASS_BASES_L,
        heights: &GRASS_HEIGHTS_L,
        leans: &GRASS_LEANS_L,
    },
];

/// Cells of one blade that carry [`material::GRASS_TIP`]: a third of the blade,
/// never fewer than [`GRASS_TIP_CELLS_PER_BLADE`].
fn blade_tip_cells(height: i32) -> i32 {
    (height / 3).max(GRASS_TIP_CELLS_PER_BLADE as i32)
}

/// One blade: a one-cell-wide stalk of `height` cells rising from its own pad
/// cell, leaning one cell outward at `lean_at` (when nonzero) and carrying
/// [`blade_tip_cells`] cells of [`material::GRASS_TIP`] at the top.
///
/// The lean direction is the dominant axis of the base offset, so a leaning
/// blade stays one cell wide: a diagonal step would need a corner cell through
/// [`set_path`] and thicken the blade to two cells, which is the cube look this
/// pass removes. The centre blade of a clump never leans.
fn grass_blade(volume: &mut DetailVolume, base: [i32; 2], height: i32, lean_at: i32) -> Result<()> {
    let lean = if base[0].abs() >= base[1].abs() {
        [base[0].signum(), 0]
    } else {
        [0, base[1].signum()]
    };
    let tip = blade_tip_cells(height);
    let mut cell = [base[0], 1, base[1]];
    volume.set(cell, material::GRASS_BLADE)?;
    for level in 1..height {
        if lean_at > 0 && level == lean_at && lean != [0, 0] {
            cell = [cell[0] + lean[0], cell[1], cell[2] + lean[1]];
        } else {
            cell = [cell[0], cell[1] + 1, cell[2]];
        }
        let m = if level >= height - tip {
            material::GRASS_TIP
        } else {
            material::GRASS_BLADE
        };
        volume.set(cell, m)?;
    }
    Ok(())
}

/// Fill the one-cell-thick pad under a clump over the rectangle from the origin
/// to `base`. The bases sit on a lattice two cells apart, so this is the
/// footprint of that lattice: the greedy mesher merges it into a single plate
/// of quads rather than the many small faces of a runner path.
fn fill_pad(volume: &mut DetailVolume, base: [i32; 2]) -> Result<()> {
    for x in 0..=base[0].abs() {
        for z in 0..=base[1].abs() {
            volume.set(
                [x * base[0].signum(), 0, z * base[1].signum()],
                material::GRASS_BLADE,
            )?;
        }
    }
    Ok(())
}

fn grass_tuft_with(id: &str, tier: &GrassTier) -> Result<DetailVolume> {
    let scale = Scale::new(LANDSCAPE_FINE_FLORA_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    // The pad is the footprint of the base lattice, one cell thick. Every blade
    // rises from its own base cell on it, so the clump is one rooted body, and
    // the greedy mesher merges the plate into a handful of quads. A runner
    // under each base instead cost four times the triangles for the same
    // silhouette, and the vertex stage is what a dense field pays for.
    for base in tier.bases {
        fill_pad(&mut volume, *base)?;
    }
    for (index, base) in tier.bases.iter().enumerate() {
        grass_blade(&mut volume, *base, tier.heights[index], tier.leans[index])?;
    }
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

pub fn grass_tuft_s(id: &str) -> Result<DetailVolume> {
    grass_tuft_with(id, &GRASS_TIERS[0])
}
pub fn grass_tuft_m(id: &str) -> Result<DetailVolume> {
    grass_tuft_with(id, &GRASS_TIERS[1])
}
pub fn grass_tuft_l(id: &str) -> Result<DetailVolume> {
    grass_tuft_with(id, &GRASS_TIERS[2])
}
// ---------------------------------------------------------------------------
// Flowers
// ---------------------------------------------------------------------------

/// Stem bases of a flower clump, and the stem heights in base order. A flower
/// is a *cluster of thin vertical elements* too: three to five one-voxel stems
/// of 8..=16 fine voxels, each carrying a bloom at its top, rooted on their own
/// pad cells. Stems are taller and sparser than a grass clump's blades so a
/// flower reads above the grass it stands in.
struct FlowerTier {
    bases: &'static [[i32; 2]],
    heights: &'static [i32],
    /// Petal arms around each bloom's top cell. `3` is three arms plus the
    /// heart, `4` is the full cross - the "full bloom" of the sized variants.
    arms: u8,
}

/// Petal arm offsets, indexed by arm count: the first three are a stable
/// non-degenerate set for small blooms, the four-arm cross is complete.
const FLOWER_ARMS: [[i32; 2]; 4] = [[1, 0], [0, 1], [0, -1], [-1, 0]];

fn flower_with(id: &str, petal: u8, tier: &FlowerTier) -> Result<DetailVolume> {
    let scale = Scale::new(LANDSCAPE_FINE_FLORA_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    // The same one-cell-thick pad a grass clump roots on.
    for base in tier.bases {
        fill_pad(&mut volume, *base)?;
    }
    for (index, base) in tier.bases.iter().enumerate() {
        let height = tier.heights[index];
        let tip = blade_tip_cells(height);
        let mut cell = [base[0], 1, base[1]];
        volume.set(cell, material::GRASS_BLADE)?;
        for level in 1..height {
            // One outward elbow below the bloom, on the dominant axis, so the
            // stem stays one cell wide exactly as a grass blade does.
            let lean = if base[0].abs() >= base[1].abs() {
                [base[0].signum(), 0]
            } else {
                [0, base[1].signum()]
            };
            if height > 4 && level == height / 2 && lean != [0, 0] {
                cell = [cell[0] + lean[0], cell[1], cell[2] + lean[1]];
            } else {
                cell = [cell[0], cell[1] + 1, cell[2]];
            }
            // The stem greens below the bloom, sharing the grass materials so
            // a meadow is one palette rather than a second one.
            let m = if level >= height - tip {
                material::GRASS_TIP
            } else {
                material::GRASS_BLADE
            };
            volume.set(cell, m)?;
        }
        // Bloom: the top cell is the heart, its arms are petals. Petal cells
        // merge with the stem above the elbow, so the bloom reads as one
        // flower head rather than a ring around a post.
        volume.set(cell, material::FLOWER_HEART)?;
        for arm in FLOWER_ARMS.iter().take(tier.arms as usize) {
            let petal_cell = [cell[0] + arm[0], cell[1], cell[2] + arm[1]];
            if volume.get(petal_cell) == material::AIR {
                volume.set(petal_cell, petal)?;
            }
        }
    }
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

/// Red: three tall stems, the largest flowers of the meadow.
const FLOWER_RED_BASES_S: [[i32; 2]; 2] = [[0, 0], [2, 0]];
const FLOWER_RED_BASES_M: [[i32; 2]; 3] = [[0, 0], [2, 0], [0, 2]];
const FLOWER_RED_BASES_L: [[i32; 2]; 3] = [[0, 0], [2, 0], [-2, 0]];
/// White: four stems, mid heights.
const FLOWER_WHITE_BASES_S: [[i32; 2]; 3] = [[0, 0], [2, 0], [0, -2]];
const FLOWER_WHITE_BASES_M: [[i32; 2]; 4] = [[0, 0], [2, 0], [0, 2], [-2, 0]];
const FLOWER_WHITE_BASES_L: [[i32; 2]; 4] = [[0, 0], [2, 0], [0, 2], [2, 2]];
/// Yellow: five short stems, a low ground-cover flower.
const FLOWER_YELLOW_BASES_S: [[i32; 2]; 4] = [[0, 0], [2, 0], [0, 2], [2, -2]];
const FLOWER_YELLOW_BASES_M: [[i32; 2]; 5] = [[0, 0], [2, 0], [0, 2], [-2, 0], [0, -2]];
const FLOWER_YELLOW_BASES_L: [[i32; 2]; 5] = [[0, 0], [2, 0], [-2, 0], [0, 2], [2, -2]];

fn flower_tier(bases: &'static [[i32; 2]], heights: &'static [i32], arms: u8) -> FlowerTier {
    FlowerTier {
        bases,
        heights,
        arms,
    }
}

pub fn flower_red_s(id: &str) -> Result<DetailVolume> {
    flower_with(
        id,
        material::FLOWER_PETAL_RED,
        &flower_tier(&FLOWER_RED_BASES_S, &[12, 10], 3),
    )
}
pub fn flower_red_m(id: &str) -> Result<DetailVolume> {
    flower_with(
        id,
        material::FLOWER_PETAL_RED,
        &flower_tier(&FLOWER_RED_BASES_M, &[16, 13, 14], 3),
    )
}
pub fn flower_red_l(id: &str) -> Result<DetailVolume> {
    flower_with(
        id,
        material::FLOWER_PETAL_RED,
        &flower_tier(&FLOWER_RED_BASES_L, &[15, 12, 14], 4),
    )
}
pub fn flower_white_s(id: &str) -> Result<DetailVolume> {
    flower_with(
        id,
        material::FLOWER_PETAL_WHITE,
        &flower_tier(&FLOWER_WHITE_BASES_S, &[9, 8, 10], 3),
    )
}
pub fn flower_white_m(id: &str) -> Result<DetailVolume> {
    flower_with(
        id,
        material::FLOWER_PETAL_WHITE,
        &flower_tier(&FLOWER_WHITE_BASES_M, &[11, 10, 12, 9], 3),
    )
}
pub fn flower_white_l(id: &str) -> Result<DetailVolume> {
    flower_with(
        id,
        material::FLOWER_PETAL_WHITE,
        &flower_tier(&FLOWER_WHITE_BASES_L, &[13, 11, 12, 10], 4),
    )
}
pub fn flower_yellow_s(id: &str) -> Result<DetailVolume> {
    flower_with(
        id,
        material::FLOWER_PETAL_YELLOW,
        &flower_tier(&FLOWER_YELLOW_BASES_S, &[8, 8, 9, 8], 3),
    )
}
pub fn flower_yellow_m(id: &str) -> Result<DetailVolume> {
    flower_with(
        id,
        material::FLOWER_PETAL_YELLOW,
        &flower_tier(&FLOWER_YELLOW_BASES_M, &[9, 8, 10, 8, 9], 3),
    )
}
pub fn flower_yellow_l(id: &str) -> Result<DetailVolume> {
    flower_with(
        id,
        material::FLOWER_PETAL_YELLOW,
        &flower_tier(&FLOWER_YELLOW_BASES_L, &[10, 9, 10, 9, 8], 4),
    )
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

/// One broadleaf limb: the height on the trunk it leaves at, the crown-plane
/// offset of its tip, and how much higher the tip is than its start. A limb is
/// walked with [`set_path`], so the bark is one connected path from the trunk to
/// the crown tip whatever the offsets are.
const BROADLEAF_LIMBS_S: [[i32; 4]; 5] = [
    [17, 6, 0, 5],
    [15, -5, 2, 4],
    [13, 2, -5, 5],
    [11, -3, -3, 6],
    [19, 3, 3, 3],
];
const BROADLEAF_LIMBS_M: [[i32; 4]; 6] = [
    [26, 9, 0, 7],
    [23, -8, 3, 6],
    [20, 3, -8, 7],
    [17, -4, -5, 8],
    [29, 5, 4, 5],
    [14, 0, 7, 6],
];
const BROADLEAF_LIMBS_L: [[i32; 4]; 7] = [
    [36, 12, 0, 9],
    [32, -11, 4, 8],
    [28, 4, -11, 9],
    [24, -6, -7, 10],
    [40, 7, 5, 7],
    [20, 0, 10, 8],
    [44, -3, -9, 6],
];

/// Leaf fronds at every limb tip (and every limb midpoint): the eight compass
/// directions, walked as one-cell arches, with the lengths below. The fronds are
/// the crown's leaf mass. Their cells are walked rather than filled as a shell,
/// so the crown is connected *by construction* while voxel-thin, and the space
/// between two fronds is a real gap a ray of light passes through - the
/// property the old solid leaf ellipsoid had none of.
const FROND_DIRS: [[i32; 2]; 8] = [
    [1, 0],
    [1, 1],
    [0, 1],
    [-1, 1],
    [-1, 0],
    [-1, -1],
    [0, -1],
    [1, -1],
];
const FROND_LENGTHS: [i32; 8] = [8, 6, 9, 6, 8, 6, 9, 6];

/// Grow one frond: a one-cell-wide arch of [`material::TREE_LEAF`] from `origin`
/// in `dir`, rising for the first half of its length and drooping back for the
/// second. Diagonal directions are staircases routed through [`set_path`], so a
/// frond is always part of the crown's single connected body.
fn leaf_frond(
    volume: &mut DetailVolume,
    origin: [i32; 3],
    dir: [i32; 2],
    length: i32,
) -> Result<()> {
    let mut prev = origin;
    for step in 1..=length {
        let rise = if step * 2 <= length { 1 } else { -1 };
        let next = [prev[0] + dir[0], prev[1] + rise, prev[2] + dir[1]];
        set_path(volume, prev, next, material::TREE_LEAF)?;
        prev = next;
    }
    Ok(())
}

/// Broadleaf tree: a 2x2 trunk, walked limbs and a crown of leaf fronds at each
/// limb tip and midpoint.
///
/// The crown is deliberately hollow: fronds radiate from the limbs and the gaps
/// between them are open, so the tree reads as a trunk carrying a leaf mass
/// rather than as a ball on a stick. Occupancy inside the crown's bounding box
/// is far below a solid's - the prototype test pins the bound.
fn broadleaf_with(
    id: &str,
    trunk_h: i32,
    limbs: &[[i32; 4]],
    frond_scale: i32,
) -> Result<DetailVolume> {
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
    for (limb_index, limb) in limbs.iter().enumerate() {
        let start = [0, limb[0], 0];
        let tip = [limb[1], limb[0] + limb[3], limb[2]];
        set_path(&mut volume, start, tip, material::TREE_BARK)?;
        // Fronds at the tip, then shorter fronds where the limb bends, so the
        // crown has two radii of leaf rather than one shell. The six directions
        // are spread by a co-prime step from the limb's own index, so the limbs
        // fill each other's gaps instead of stacking six identical rosettes.
        // The bend cell is the limb's elbow, which `set_path` always occupies:
        // a frond must sprout from bark, not from the air beside a diagonal.
        let elbow = [tip[0], start[1], tip[2]];
        for k in 0..6 {
            let index = (limb_index * 3 + k * 5) % FROND_DIRS.len();
            let length = FROND_LENGTHS[index] * frond_scale / 4;
            leaf_frond(&mut volume, tip, FROND_DIRS[index], length)?;
        }
        for k in 0..3 {
            let index = (limb_index * 3 + k * 5 + 2) % FROND_DIRS.len();
            let length = (FROND_LENGTHS[index] * frond_scale / 8).max(3);
            leaf_frond(&mut volume, elbow, FROND_DIRS[index], length)?;
        }
    }
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

pub fn tree_broadleaf_s(id: &str) -> Result<DetailVolume> {
    broadleaf_with(id, 22, &BROADLEAF_LIMBS_S, 3)
}
pub fn tree_broadleaf_m(id: &str) -> Result<DetailVolume> {
    broadleaf_with(id, 34, &BROADLEAF_LIMBS_M, 4)
}
pub fn tree_broadleaf_l(id: &str) -> Result<DetailVolume> {
    broadleaf_with(id, 46, &BROADLEAF_LIMBS_L, 5)
}

/// Conifer needle rings: (height, radius), shrinking towards the top.
const CONIFER_RINGS_S: [(i32, i32); 5] = [(6, 4), (9, 3), (12, 3), (15, 2), (18, 1)];
const CONIFER_RINGS_M: [(i32, i32); 6] = [(8, 5), (12, 4), (16, 4), (20, 3), (24, 2), (28, 1)];
const CONIFER_RINGS_L: [(i32, i32); 7] = [
    (10, 6),
    (15, 5),
    (20, 5),
    (25, 4),
    (30, 3),
    (35, 2),
    (40, 1),
];

/// One needle ring at height `y` and radius `r`, filled outward from the trunk.
///
/// Cells are visited in order of increasing `|x| + |z|` and a cell is only
/// placed when it has an occupied neighbour nearer the axis, so the ring is
/// connected *by construction* however the gap pattern falls. The four axis
/// spokes are always solid: they are the ring's structure and they meet the
/// trunk. Quadrant cells are dashed by an integer pattern, which is where the
/// light comes through a conifer crown.
fn needle_ring(volume: &mut DetailVolume, y: i32, r: i32) -> Result<()> {
    let mut cells: Vec<[i32; 2]> = Vec::new();
    for x in -r..=r {
        for z in -r..=r {
            if x * x + z * z <= r * r {
                cells.push([x, z]);
            }
        }
    }
    // BFS order from the axis: deterministic and independent of the hash of a
    // coordinate pair.
    cells.sort_by_key(|[x, z]| (x.abs() + z.abs(), x.abs(), z.abs(), *x, *z));
    for [x, z] in cells {
        if x != 0 && z != 0 {
            // Dashed quadrants: roughly two of every three cells stay.
            if (x + 2 * z + y) % 3 == 0 {
                continue;
            }
            // Keep the ring's outer edge rather than every interior quadrant
            // cell, so a ring is an annulus with spokes, not a solid disc.
            let inner = (r - 1).max(1);
            if x * x + z * z < inner * inner && (x + z + y) % 2 != 0 {
                continue;
            }
        }
        let cell = [x, y, z];
        if volume.get(cell) != material::AIR {
            continue;
        }
        let has_neighbour = [[1, 0], [-1, 0], [0, 1], [0, -1]]
            .iter()
            .any(|[dx, dz]| volume.get([x + dx, y, z + dz]) != material::AIR);
        if has_neighbour {
            volume.set(cell, material::TREE_NEEDLE)?;
        }
    }
    Ok(())
}

/// Conifer: a straight 2x2 trunk with stacked needle rings that shrink towards
/// the top and a needle tip. Rings are laid over air only, so the trunk shows
/// through and every ring merges with it.
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
        needle_ring(&mut volume, y, radius)?;
    }
    let tip = [0, trunk_h + 1, 0];
    if volume.get(tip) == material::AIR {
        volume.set(tip, material::TREE_NEEDLE)?;
    }
    debug_assert!(is_single_body(&volume));
    Ok(volume)
}

pub fn tree_conifer_s(id: &str) -> Result<DetailVolume> {
    conifer_with(id, 18, &CONIFER_RINGS_S)
}
pub fn tree_conifer_m(id: &str) -> Result<DetailVolume> {
    conifer_with(id, 28, &CONIFER_RINGS_M)
}
pub fn tree_conifer_l(id: &str) -> Result<DetailVolume> {
    conifer_with(id, 40, &CONIFER_RINGS_L)
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
