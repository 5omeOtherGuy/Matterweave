//! Wetland support flora: three non-fungal prototypes (Muse worker).
//!
//! This module is additive only: the existing [`crate::flora`] catalogue and the
//! [`crate::material`] palette are untouched. The bracket/shelf fungus is
//! RESERVED (Hy4/lead, separate file) and is NOT implemented here.
//!
//! # Material-policy reuse (explicit, no palette changes)
//!
//! No new material IDs were added. All woody structure reuses
//! [`material::FLORA_FUNNEL_STIPE`] (ID 30, `Collision`, warm tan
//! `[0.80, 0.68, 0.47]`): it is an existing collidable stem-wood-tone solid,
//! so a trunk built from it stays a gameplay wall while reading as wood.
//! `BANK_STONE` was deliberately NOT used for wood: it is gray terrain stone
//! and would misread as rock outcrops. All foliage, needles, pads, petals and
//! reed-like stems reuse existing `Decorative` leaf IDs (38-47); they are
//! queryable source geometry but never collision walls. The luminous accent
//! [`material::FLORA_LUMEN_DOT`] (decorative leaf accent) is used only on the
//! lily flower heart, never inside collidable wood.
//!
//! # Species and scales
//!
//! - `twisted_shrub`: twisted woody small tree at 12.5 cm cells. Collidable
//!   trunk/branches, decorative leaf crowns.
//! - `horsetail`: tall segmented spire plant at 6.25 cm cells. Fully
//!   decorative; node bands, whorled needles, spore cone tip, two side shoots.
//! - `marsh_lily`: broad water-lily-like plant at 12.5 cm cells. Fully
//!   decorative; rhizome runner feet at y = 0, arching stems, floating notched
//!   pads, one bud/flower. Intended for shallow-water placement: the pads float
//!   above the bed and the dry-moss [`crate::flora::instance_support`] query
//!   does NOT apply to it; water placement is recorded in the gallery manifest.
//!
//! Every prototype is one six-connected body rooted at local y = 0,
//! deterministic (id-independent integer construction), and far below the
//! 33,288-cell mesh preflight cap.

use crate::{material, DetailVolume, Result, Scale};
use std::collections::BTreeSet;

/// Wetland support-flora prototype IDs built by this module.
pub const WETLAND_FLORA_SPECIES: [&str; 3] = ["twisted_shrub", "horsetail", "marsh_lily"];
/// Explicit fine scale for the woody shrub and lily (12.5 cm cells).
pub const WETLAND_LEAF_SCALE_M: f32 = 0.125;
/// Explicit fine scale for the horsetail spire (6.25 cm cells).
pub const WETLAND_SPIRE_SCALE_M: f32 = 0.0625;

/// Walk from `from` to `to` one axis at a time (x, then z, then y), filling
/// every intermediate air cell, so a rasterised trunk/branch/stem path is
/// six-connected instead of a chain of diagonal touches. Occupied cells are
/// left alone, so paths merge into existing wood instead of overwriting it.
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

/// Six-connected component count over occupied cells (worker-local copy; the
/// existing flora module is untouched).
fn connected_components(volume: &DetailVolume) -> usize {
    let cells: BTreeSet<[i32; 3]> = volume.iter_cells().map(|(c, _)| c).collect();
    let mut seen: BTreeSet<[i32; 3]> = BTreeSet::new();
    let mut count = 0;
    for start in &cells {
        if seen.contains(start) {
            continue;
        }
        count += 1;
        let mut stack = vec![*start];
        seen.insert(*start);
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
    }
    count
}

// ---------------------------------------------------------------------------
// Twisted woody shrub / small tree
// ---------------------------------------------------------------------------

/// Trunk centre x-offset per height: an S-twist with steps of at most one cell
/// so consecutive trunk discs always overlap (six-connected by construction).
const SHRUB_TWIST: [i32; 15] = [0, 0, 1, 1, 2, 2, 1, 0, -1, -1, 0, 1, 1, 0, 0];
/// Branches: (takeoff height, direction [x, z], length). Distinct heights,
/// directions and lengths give bounded variation without noise.
const SHRUB_BRANCHES: [(i32, [i32; 2], i32); 4] = [
    (5, [1, 0], 6),
    (7, [-1, 0], 7),
    (9, [0, 1], 6),
    (11, [0, -1], 5),
];

/// Fill an ellipsoid crown of leaf around `centre`; the branch-tip cell that
/// seeds it is always inside the radii, so the crown merges with the wood.
/// Interior is blade, the equator ring carries rib veins, scattered surface
/// cells are contrasting tips.
fn leaf_crown(volume: &mut DetailVolume, centre: [i32; 3], radii: [i32; 3]) -> Result<()> {
    let [cx, cy, cz] = centre;
    let [rx, ry, rz] = radii;
    for x in -rx..=rx {
        for y in -ry..=ry {
            for z in -rz..=rz {
                let d = (x * x) as f32 / ((rx * rx) as f32)
                    + (y * y) as f32 / ((ry * ry) as f32)
                    + (z * z) as f32 / ((rz * rz) as f32);
                if d > 1.0 {
                    continue;
                }
                let cell = [cx + x, cy + y, cz + z];
                if volume.get(cell) != material::AIR {
                    continue;
                }
                let surface = d > 0.55;
                let chosen = if y == 0 && (x == 0 || z == 0) {
                    material::FLORA_FROND_RIB
                } else if surface && (x * 2 + z * 3 + y).rem_euclid(5) == 0 {
                    material::FLORA_ROSETTE_LEAF
                } else {
                    material::FLORA_FROND_BLADE
                };
                volume.set(cell, chosen)?;
            }
        }
    }
    Ok(())
}

/// Twisted woody shrub/small tree at 12.5 cm cells: S-twisted collidable trunk
/// (root disc at y = 0), four tapered arching branches and five decorative
/// leaf crowns. Branches taper (two cells wide at the takeoff, one cell
/// afterwards) and rise as they extend, so they are limbs, not cylinders.
pub fn twisted_shrub(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(WETLAND_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    let wood = material::FLORA_FUNNEL_STIPE;
    // Root disc at y = 0: the whole ground-contact footprint.
    for x in -2..=2 {
        for z in -2..=2 {
            if x * x + z * z <= 4 {
                volume.set([x, 0, z], wood)?;
            }
        }
    }
    // Twisted trunk: thick foot, slim waist, slight swell under the crown.
    for (y, tx) in SHRUB_TWIST.into_iter().enumerate() {
        let y = y as i32 + 1;
        let radius = match y {
            1 | 2 => 2,
            13 | 14 => 1,
            _ => 1,
        };
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x * x + z * z <= radius * radius {
                    volume.set([tx + x, y, z], wood)?;
                }
            }
        }
        // Waist knot on the second bend: a real cross-section change, not a
        // straight pipe.
        if y == 6 {
            volume.set([tx + 1, y, 1], wood)?;
            volume.set([tx + 1, y, -1], wood)?;
        }
    }
    // Branches with crowns at their tips.
    for (h, dir, len) in SHRUB_BRANCHES {
        let base = [SHRUB_TWIST[(h - 1) as usize], h, 0];
        let mut prev = base;
        for step in 1..=len {
            let cell = [
                base[0] + dir[0] * step,
                h + step / 2,
                base[2] + dir[1] * step,
            ];
            set_path(&mut volume, prev, cell, wood)?;
            // Taper: only the first two steps carry a side cell.
            if step <= 2 {
                let side = [cell[0] - dir[1], cell[1], cell[2] + dir[0]];
                if volume.get(side) == material::AIR {
                    volume.set(side, wood)?;
                }
            }
            prev = cell;
        }
        leaf_crown(&mut volume, [prev[0], prev[1] + 1, prev[2]], [3, 2, 3])?;
    }
    // Trunk-top crown.
    let top = SHRUB_TWIST[14];
    leaf_crown(&mut volume, [top, 17, 0], [3, 2, 3])?;
    debug_assert_eq!(connected_components(&volume), 1);
    Ok(volume)
}

// ---------------------------------------------------------------------------
// Tall horsetail / spire plant
// ---------------------------------------------------------------------------

/// Node heights on the main spire (joint bands) and side-shoot (base, height).
const HORSETAIL_NODES: [i32; 5] = [4, 9, 14, 19, 24];
const HORSETAIL_SPIRE_TOP: i32 = 28;
const HORSETAIL_SHOOTS: [([i32; 2], i32); 2] = [([4, 2], 14), ([-4, -2], 18)];
const HORSETAIL_WHORL: [[i32; 2]; 8] = [
    [1, 0],
    [1, 1],
    [0, 1],
    [-1, 1],
    [-1, 0],
    [-1, -1],
    [0, -1],
    [1, -1],
];

/// One whorl of angled needles around `(cx, node, cz)`: each needle walks
/// outward one axis at a time and lifts every second step, so diagonal needles
/// are connected arches, not isolated voxels.
fn whorl(volume: &mut DetailVolume, cx: i32, node: i32, cz: i32, length: i32) -> Result<()> {
    for dir in HORSETAIL_WHORL {
        let mut cell = [cx, node, cz];
        for k in 1..=length {
            if dir[0] != 0 {
                cell[0] += dir[0];
                if volume.get(cell) == material::AIR {
                    volume.set(cell, material::FLORA_REED_LEAF)?;
                }
            }
            if dir[1] != 0 {
                cell[2] += dir[1];
                if volume.get(cell) == material::AIR {
                    volume.set(cell, material::FLORA_REED_LEAF)?;
                }
            }
            if k % 2 == 0 {
                cell[1] += 1;
                if volume.get(cell) == material::AIR {
                    volume.set(cell, material::FLORA_REED_LEAF)?;
                }
            }
        }
    }
    Ok(())
}

/// Spore cone tip above `top`: stacked tapering plume discs with a rib collar.
fn spore_cone(volume: &mut DetailVolume, cx: i32, top: i32, cz: i32) -> Result<()> {
    for (dy, radius) in [(1, 2), (2, 2), (3, 1), (4, 1), (5, 0)] {
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x * x + z * z <= radius * radius
                    && volume.get([cx + x, top + dy, cz + z]) == material::AIR
                {
                    volume.set([cx + x, top + dy, cz + z], material::FLORA_REED_PLUME)?;
                }
            }
        }
    }
    for x in -1..=1 {
        for z in -1..=1 {
            if volume.get([cx + x, top, cz + z]) == material::AIR {
                volume.set([cx + x, top, cz + z], material::FLORA_FROND_RIB)?;
            }
        }
    }
    Ok(())
}

/// Tall horsetail/spire plant at 6.25 cm cells (fully decorative): one dominant
/// segmented spire (~1.9 m) with joint-band rings wider than the internodes,
/// whorled needle rings at each node, a spore cone tip and two shorter side
/// shoots joined by y = 0 runners. Reads as a jointed spire, not a reed bed:
/// a single vertical accent instead of nine equal culms.
pub fn horsetail(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(WETLAND_SPIRE_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    let stem = material::FLORA_REED_STEM;
    // Rhizome foot: compact disc at y = 0 plus runners to the side shoots.
    for x in -2..=2 {
        for z in -2..=2 {
            if x * x + z * z <= 4 {
                volume.set([x, 0, z], stem)?;
            }
        }
    }
    for (base, _) in HORSETAIL_SHOOTS {
        let (mut cx, mut cz) = (0, 0);
        while cx != base[0] {
            cx += (base[0] - cx).signum();
            if volume.get([cx, 0, cz]) == material::AIR {
                volume.set([cx, 0, cz], stem)?;
            }
        }
        while cz != base[1] {
            cz += (base[1] - cz).signum();
            if volume.get([cx, 0, cz]) == material::AIR {
                volume.set([cx, 0, cz], stem)?;
            }
        }
    }
    // Main spire: slim ribbed segments (plus-shaped cross-section) between nodes.
    for y in 1..=HORSETAIL_SPIRE_TOP {
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            if volume.get([dx, y, dz]) == material::AIR {
                volume.set([dx, y, dz], stem)?;
            }
        }
    }
    for node in HORSETAIL_NODES {
        // Joint band: full disc of radius 2, visibly wider than the internode.
        for x in -2..=2 {
            for z in -2..=2 {
                if x * x + z * z <= 4 && volume.get([x, node, z]) == material::AIR {
                    volume.set([x, node, z], material::FLORA_FROND_RIB)?;
                }
            }
        }
        whorl(&mut volume, 0, node, 0, 5)?;
    }
    spore_cone(&mut volume, 0, HORSETAIL_SPIRE_TOP, 0)?;
    // Side shoots: slimmer, fewer nodes, smaller whorls and cone tips.
    for (base, height) in HORSETAIL_SHOOTS {
        for y in 1..=height {
            if volume.get([base[0], y, base[1]]) == material::AIR {
                volume.set([base[0], y, base[1]], stem)?;
            }
        }
        let node = height / 2;
        for x in -1..=1 {
            for z in -1..=1 {
                if x * x + z * z <= 2
                    && volume.get([base[0] + x, node, base[1] + z]) == material::AIR
                {
                    volume.set([base[0] + x, node, base[1] + z], material::FLORA_FROND_RIB)?;
                }
            }
        }
        whorl(&mut volume, base[0], node, base[1], 3)?;
        spore_cone(&mut volume, base[0], height, base[1])?;
    }
    debug_assert_eq!(connected_components(&volume), 1);
    Ok(volume)
}

// ---------------------------------------------------------------------------
// Broad water-lily / lotus-like plant
// ---------------------------------------------------------------------------

/// Pads: (runner x, pad-centre offset [x, z], radius). Alternating radii and
/// offsets give a spread colony, not a row.
const LILY_PADS: [(i32, [i32; 2], i32); 5] = [
    (-6, [-1, 2], 4),
    (-3, [1, -4], 3),
    (0, [1, 5], 3),
    (3, [2, -5], 3),
    (6, [1, 3], 4),
];
const LILY_PAD_Y: i32 = 2;

/// Broad water-lily-like plant at 12.5 cm cells (fully decorative): a rhizome
/// runner at y = 0, arching stems to five floating notched pads at y = 2 and
/// one bud/flower with a luminous heart. Pads are flat discs (width far
/// exceeds height) with a wedge slit; the flower is layered petals, not a blob.
pub fn marsh_lily(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(WETLAND_LEAF_SCALE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    let stem = material::FLORA_REED_STEM;
    // Rhizome runner along x at y = 0: the whole ground-contact footprint.
    for x in -7..=7 {
        if volume.get([x, 0, 0]) == material::AIR {
            volume.set([x, 0, 0], stem)?;
        }
    }
    for (index, (rx, off, radius)) in LILY_PADS.into_iter().enumerate() {
        let centre = [rx + off[0], LILY_PAD_Y, off[1]];
        // Arching stem from the runner to the pad underside.
        set_path(
            &mut volume,
            [rx, 0, 0],
            [centre[0], centre[1] - 1, centre[2]],
            stem,
        )?;
        volume.set(
            [centre[0], centre[1] - 1, centre[2]],
            material::FLORA_ROSETTE_HEART,
        )?;
        // Floating pad: flat disc with a wedge slit (dx > 0, dz == 0 stays air),
        // blade rim, leaf interior, young heart at the stem joint.
        for x in -radius..=radius {
            for z in -radius..=radius {
                let r2 = x * x + z * z;
                if r2 > radius * radius {
                    continue;
                }
                if x > 0 && z == 0 {
                    continue; // Notch slit open to the rim.
                }
                let cell = [centre[0] + x, LILY_PAD_Y, centre[2] + z];
                if volume.get(cell) != material::AIR {
                    continue;
                }
                let chosen = if x == 0 && z == 0 {
                    material::FLORA_ROSETTE_HEART
                } else if r2 > (radius - 1) * (radius - 1) {
                    material::FLORA_FROND_BLADE
                } else {
                    material::FLORA_ROSETTE_LEAF
                };
                volume.set(cell, chosen)?;
            }
        }
        // One flower on the middle pad: short stalk, heart cup, spot petals,
        // luminous heart dot.
        if index == 2 {
            volume.set([centre[0], 3, centre[2]], material::FLORA_ROSETTE_HEART)?;
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                volume.set(
                    [centre[0] + dx, 4, centre[2] + dz],
                    material::FLORA_ROSETTE_SPOT,
                )?;
            }
            volume.set([centre[0], 4, centre[2]], material::FLORA_LUMEN_DOT)?;
            volume.set([centre[0], 5, centre[2]], material::FLORA_ROSETTE_SPOT)?;
        }
        // A second smaller bud on the last pad: stalk plus four petals.
        if index == 4 {
            volume.set([centre[0], 3, centre[2]], material::FLORA_ROSETTE_HEART)?;
            volume.set([centre[0], 4, centre[2]], material::FLORA_ROSETTE_SPOT)?;
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                volume.set(
                    [centre[0] + dx, 4, centre[2] + dz],
                    material::FLORA_ROSETTE_SPOT,
                )?;
            }
        }
    }
    debug_assert_eq!(connected_components(&volume), 1);
    Ok(volume)
}

/// Build one wetland support-flora prototype by ID.
pub fn wetland_prototype(id: &str) -> Result<DetailVolume> {
    match id {
        "twisted_shrub" => twisted_shrub(id),
        "horsetail" => horsetail(id),
        "marsh_lily" => marsh_lily(id),
        _ => Err(crate::DetailError::UnknownPrototype(id.to_string())),
    }
}
