//! Deterministic content fixtures. All generation is integer-only, so output is
//! identical across hosts and runs for a given seed and generator version.

use crate::{
    material, DetailScene, DetailVolume, Result, Scale, Transform, Yaw, SCALE_FINE_M, SCALE_TILE_M,
};

pub const FIXTURE_GENERATOR_VERSION: u32 = 1;
/// 16 m tile at 25 cm cells, centred on the local origin: cells `-32..32`.
pub const TILE_EDGE_CELLS: i32 = 64;
pub const MUSHROOM_SCALE: f32 = SCALE_FINE_M;

fn hash(seed: u64, x: i32, z: i32) -> u64 {
    let mut value = seed
        ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (z as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Bilinear integer value noise over an 8-cell lattice, matching the core
/// generator's style so terrain reads consistently at either scale.
fn relief(seed: u64, x: i32, z: i32) -> i32 {
    let (cx, cz) = (x.div_euclid(8), z.div_euclid(8));
    let (fx, fz) = (x.rem_euclid(8), z.rem_euclid(8));
    let noise = |dx, dz| (hash(seed, cx + dx, cz + dz) % 5) as i32;
    let a = noise(0, 0) * (8 - fx) + noise(1, 0) * fx;
    let b = noise(0, 1) * (8 - fx) + noise(1, 1) * fx;
    (a * (8 - fz) + b * fz) / 64
}

/// Surface height in cells for the tile fixture: a bank rising to +x, an eroded
/// creek channel near `z = -8` and local noise relief.
fn tile_height(seed: u64, x: i32, z: i32) -> i32 {
    let bank = (x + 32) / 12;
    let channel = (6 - (z + 8).abs()).max(0);
    relief(seed, x, z) + bank - channel
}

/// Water surface height in cells; cells at or below this in the channel are water.
pub const TILE_WATER_LEVEL_CELLS: i32 = 0;

/// 16 m x 16 m detail terrain tile at 25 cm cells with negative coordinates,
/// uneven bank relief and a water-filled creek.
///
/// Water cells use [`material::WATER`], whose policy is liquid: queryable source
/// data, never a collision wall.
pub fn terrain_detail_tile(id: &str, seed: u64) -> Result<DetailVolume> {
    let scale = Scale::new(SCALE_TILE_M)?;
    let mut volume = DetailVolume::new(id, scale);
    let half = TILE_EDGE_CELLS / 2;
    for x in -half..half {
        for z in -half..half {
            let height = tile_height(seed, x, z);
            for y in (height - 5)..=height {
                let material = if y == height {
                    if height <= TILE_WATER_LEVEL_CELLS {
                        material::DETAIL_SOIL
                    } else {
                        material::MOSS_TURF
                    }
                } else if y >= height - 2 {
                    material::DETAIL_SOIL
                } else {
                    material::BANK_STONE
                };
                volume.set([x, y, z], material)?;
            }
            for y in (height + 1)..=TILE_WATER_LEVEL_CELLS {
                volume.set([x, y, z], material::WATER)?;
            }
        }
    }
    Ok(volume)
}

/// Sixteen integer blade directions scaled by 16, so gill placement needs no
/// floating point and is bit-identical on every target.
const BLADES: [[i32; 2]; 16] = [
    [16, 0],
    [15, 6],
    [11, 11],
    [6, 15],
    [0, 16],
    [-6, 15],
    [-11, 11],
    [-15, 6],
    [-16, 0],
    [-15, -6],
    [-11, -11],
    [-6, -15],
    [0, -16],
    [6, -15],
    [11, -11],
    [15, -6],
];

/// Original parasol mushroom at 6.25 cm cells: about 1.4 m tall with a flared
/// crown, a downturned rim, radial underside gills and a distinct tapering
/// stipe joined to the cap by a solid collar.
pub fn parasol_mushroom(id: &str) -> Result<DetailVolume> {
    let scale = Scale::new(SCALE_FINE_M).expect("fine scale is valid");
    let mut volume = DetailVolume::new(id, scale);

    // Stipe: flared foot, slim waist, slight swell under the cap.
    for y in 0..=15 {
        let radius = match y {
            0..=1 => 3,
            2..=3 => 2,
            4..=12 => 1,
            _ => 2,
        };
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x * x + z * z <= radius * radius {
                    volume.set([x, y, z], material::MUSHROOM_STIPE)?;
                }
            }
        }
    }

    // Collar at the cap underside keeps stipe, gills and cap one connected body.
    for x in -2..=2 {
        for z in -2..=2 {
            if x * x + z * z <= 4 {
                volume.set([x, 16, z], material::MUSHROOM_STIPE)?;
            }
        }
    }

    // Downturned rim ring at the widest cap layer.
    for x in -9..=9 {
        for z in -9..=9 {
            let r2 = x * x + z * z;
            if (64..=81).contains(&r2) {
                volume.set([x, 16, z], material::MUSHROOM_RIM)?;
            }
        }
    }

    // Radial gill blades under the cap, between collar and rim.
    for blade in BLADES {
        for r in 3..=7 {
            let cell = [blade[0] * r / 16, 16, blade[1] * r / 16];
            if volume.get(cell) == material::AIR {
                volume.set(cell, material::MUSHROOM_GILL)?;
            }
        }
    }

    // Cap crown: broad underside disc narrowing to a raised umbo.
    for (y, radius) in [(17, 9), (18, 8), (19, 6), (20, 4), (21, 2)] {
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x * x + z * z <= radius * radius {
                    let material = if y == 17 && x * x + z * z > 64 {
                        material::MUSHROOM_RIM
                    } else {
                        material::MUSHROOM_CAP
                    };
                    volume.set([x, y, z], material)?;
                }
            }
        }
    }
    Ok(volume)
}

/// Foot radius of [`parasol_mushroom`] in its own 6.25 cm cells.
pub const MUSHROOM_FOOT_RADIUS_CELLS: i32 = 3;
/// Tile columns the foot can touch on either side of its centre column, derived
/// from the foot radius in metres and the tile cell size.
const FOOT_COLUMN_REACH: i32 = 1;

/// Highest occupied cell of a tile column, with its material.
pub fn column_top(volume: &DetailVolume, x: i32, z: i32) -> Option<(i32, u8)> {
    let (min, max) = volume.cell_bounds()?;
    column_top_in(volume, x, z, (min[1], max[1]))
}

/// Same query with a precomputed vertical range, so a placement search does not
/// rescan the whole volume for every column.
fn column_top_in(volume: &DetailVolume, x: i32, z: i32, y_range: (i32, i32)) -> Option<(i32, u8)> {
    (y_range.0..=y_range.1).rev().find_map(|y| {
        let material = volume.get([x, y, z]);
        (material != material::AIR).then_some((y, material))
    })
}

/// A column is a valid mushroom footing when its whole foot footprint shares one
/// dry surface height: same top cell, no water on top and no water in the column
/// surface. Placement heights are derived from these authoritative queries; no
/// arbitrary vertical offset is applied anywhere.
fn footing_height(tile: &DetailVolume, x: i32, z: i32, y_range: (i32, i32)) -> Option<i32> {
    let (top, material) = column_top_in(tile, x, z, y_range)?;
    if material != material::MOSS_TURF || top <= TILE_WATER_LEVEL_CELLS {
        return None;
    }
    for dx in -FOOT_COLUMN_REACH..=FOOT_COLUMN_REACH {
        for dz in -FOOT_COLUMN_REACH..=FOOT_COLUMN_REACH {
            let (neighbour_top, neighbour_material) = column_top_in(tile, x + dx, z + dz, y_range)?;
            if neighbour_top != top || neighbour_material != material::MOSS_TURF {
                return None;
            }
        }
    }
    Some(top)
}

/// Deterministic search seeds spread over the tile; each one takes the nearest
/// valid footing within a bounded ring, so placements follow the terrain instead
/// of being nudged by hand.
const PLACEMENT_SEEDS: [[i32; 2]; 6] =
    [[-24, -20], [-12, 8], [-4, -6], [6, 14], [14, -12], [20, 4]];
const PLACEMENT_SEARCH: i32 = 6;
const PLACEMENT_YAWS: [Yaw; 4] = [Yaw::Deg0, Yaw::Deg90, Yaw::Deg180, Yaw::Deg270];

/// Reusable gallery scene: one detail tile plus ground-supported mushroom
/// instances. This is deliberately a *sparse* demonstration scene, not a dense
/// showcase and not a density claim; composition lives here so a native adapter
/// or test can build the identical scene without going through the example.
pub fn gallery_scene(seed: u64) -> Result<DetailScene> {
    let tile = terrain_detail_tile("terrain_tile_16m", seed)?;
    let mushroom = parasol_mushroom("parasol_mushroom")?;
    let cell = tile.scale().metres();
    let (min, max) = tile.cell_bounds().expect("tile is not empty");
    let y_range = (min[1], max[1]);

    let mut placements: Vec<([f32; 3], Yaw)> = Vec::new();
    for (index, [sx, sz]) in PLACEMENT_SEEDS.into_iter().enumerate() {
        let mut best: Option<(i32, i32, i32)> = None;
        for dx in -PLACEMENT_SEARCH..=PLACEMENT_SEARCH {
            for dz in -PLACEMENT_SEARCH..=PLACEMENT_SEARCH {
                let (x, z) = (sx + dx, sz + dz);
                let candidate = [(x as f32 + 0.5) * cell, (z as f32 + 0.5) * cell];
                if footing_is_separate(&placements, candidate)
                    && footing_height(&tile, x, z, y_range).is_some()
                {
                    let distance = dx.abs() + dz.abs();
                    if best.is_none_or(|(previous, _, _)| distance < previous) {
                        best = Some((distance, x, z));
                    }
                }
            }
        }
        let Some((_, x, z)) = best else { continue };
        // The surface cell top is the support plane; the prototype's own y = 0
        // is its foot, so the translation is the derived height, not an offset.
        let top = footing_height(&tile, x, z, y_range).expect("footing revalidated");
        placements.push((
            [
                (x as f32 + 0.5) * cell,
                (top + 1) as f32 * cell,
                (z as f32 + 0.5) * cell,
            ],
            PLACEMENT_YAWS[index % PLACEMENT_YAWS.len()],
        ));
    }

    let mut scene = DetailScene::new();
    scene.add_prototype(tile)?;
    scene.add_prototype(mushroom)?;
    scene.place("tile-0", "terrain_tile_16m", Transform::identity())?;
    for (index, (translation, yaw)) in placements.into_iter().enumerate() {
        scene.place(
            format!("parasol-{index}"),
            "parasol_mushroom",
            Transform::new(translation, yaw)?,
        )?;
    }
    Ok(scene)
}

// Keep bottom-foot bounding boxes disjoint in both horizontal axes. Do this
// during candidate search so a blocked nearest spot still permits another one.
fn footing_is_separate(placed: &[([f32; 3], Yaw)], candidate: [f32; 2]) -> bool {
    let diameter = (2 * MUSHROOM_FOOT_RADIUS_CELLS + 1) as f32 * MUSHROOM_SCALE;
    placed.iter().all(|(t, _)| {
        (t[0] - candidate[0]).abs() >= diameter || (t[2] - candidate[1]).abs() >= diameter
    })
}

#[cfg(test)]
mod placement_tests {
    use super::*;
    #[test]
    fn spacing_checks_both_axes() {
        let placed = [([0.0, 0.0, 0.0], Yaw::Deg0)];
        assert!(footing_is_separate(&placed, [0.0, 0.5]));
        assert!(footing_is_separate(&placed, [0.5, 0.0]));
        assert!(!footing_is_separate(&placed, [0.25, 0.0]));
        assert!(!footing_is_separate(&placed, [0.0, 0.25]));
    }
}
