//! Core voxel material identity: names, vertex colours and physical policy.
//!
//! Material IDs are a serialized contract: a saved world stores them directly, so
//! an existing ID never changes meaning. IDs `1..=7` are the original core
//! palette. Landscape additions start at 51 to leave the detail crate's
//! `10..=48` palette untouched, and `0` is always air.
//!
//! Physical classification lives here rather than in the physics adapter so
//! collision, meshing and generation agree on one definition. `is_solid` is the
//! occupancy/collision predicate; water is deliberately not solid and is never
//! stored in the authoritative world (see [`crate::landscape`]).

pub const AIR: u8 = 0;
/// Grassy or mossy ground cover. Original core ID; `landscape` uses it as grass.
pub const MOSS: u8 = 1;
pub const SOIL: u8 = 2;
pub const STONE: u8 = 3;
pub const SAND: u8 = 4;
pub const WOOD: u8 = 5;
pub const CANOPY: u8 = 6;
pub const MINERAL: u8 = 7;

/// Snow cover on cold or high ground.
pub const SNOW: u8 = 51;
/// Ice sheet; solid to the player, unlike water.
pub const ICE: u8 = 52;
/// Water surface. Derived only: never a stored voxel, never a collision wall.
pub const WATER: u8 = 53;
/// Coarse gravel on steep slopes and river beds.
pub const GRAVEL: u8 = 54;
/// Dry clay under sand and in badlands.
pub const CLAY: u8 = 55;

/// Every material the core palette defines. Used by palette and coverage tests.
pub const PALETTE: [u8; 13] = [
    AIR, MOSS, SOIL, STONE, SAND, WOOD, CANOPY, MINERAL, SNOW, ICE, WATER, GRAVEL, CLAY,
];

/// Stable name of a material, for diagnostics and saved-world reporting.
pub fn name(material: u8) -> &'static str {
    match material {
        AIR => "air",
        MOSS => "moss",
        SOIL => "soil",
        STONE => "stone",
        SAND => "sand",
        WOOD => "wood",
        CANOPY => "canopy",
        MINERAL => "mineral",
        SNOW => "snow",
        ICE => "ice",
        WATER => "water",
        GRAVEL => "gravel",
        CLAY => "clay",
        _ => "unknown",
    }
}

/// Vertex colour of a material. The original seven values are unchanged; a
/// world saved before this module existed still renders with the same colours.
pub fn color(material: u8) -> [f32; 3] {
    match material {
        MOSS => [0.29, 0.48, 0.27],
        SOIL => [0.35, 0.24, 0.17],
        STONE => [0.47, 0.51, 0.52],
        SAND => [0.72, 0.65, 0.46],
        WOOD => [0.30, 0.20, 0.13],
        CANOPY => [0.19, 0.38, 0.28],
        MINERAL => [0.42, 0.77, 0.72],
        SNOW => [0.86, 0.88, 0.92],
        ICE => [0.72, 0.84, 0.90],
        // Exactly the detail crate's water colour; the renderer's water pass and
        // the shader's legacy colour test both key on this value.
        WATER => [0.16, 0.34, 0.42],
        GRAVEL => [0.44, 0.43, 0.41],
        CLAY => [0.55, 0.42, 0.32],
        _ => [0.69, 0.46, 0.33],
    }
}

/// Whether a material occupies space for collision, support and terrain queries.
/// Water is the only non-solid material: a player swims or wades through it.
pub fn is_solid(material: u8) -> bool {
    material != AIR && material != WATER
}

/// Whether a material is a liquid surface rather than walkable ground.
pub fn is_liquid(material: u8) -> bool {
    material == WATER
}

/// Whether the greedy mesher may merge this material as a fully covered surface.
/// Water is not opaque, so the derived water surface is meshed by its own path
/// and never hidden by a neighbouring cell.
pub fn is_opaque(material: u8) -> bool {
    material != AIR && material != WATER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_entry_has_a_name_and_colour() {
        for material in PALETTE {
            assert!(
                name(material) != "unknown",
                "material {material} has no name"
            );
            let colour = color(material);
            assert!(colour.iter().all(|v| v.is_finite()), "{material} colour");
            assert!(colour.iter().all(|v| (0.0..=1.0).contains(v)), "{material}");
        }
        assert_eq!(name(200), "unknown");
    }

    #[test]
    fn policy_is_explicit_for_air_water_and_ground() {
        assert!(!is_solid(AIR) && !is_liquid(AIR) && !is_opaque(AIR));
        assert!(!is_solid(WATER) && is_liquid(WATER) && !is_opaque(WATER));
        for solid in [
            MOSS, SOIL, STONE, SAND, WOOD, CANOPY, MINERAL, SNOW, ICE, GRAVEL, CLAY,
        ] {
            assert!(is_solid(solid), "{} must be solid", name(solid));
            assert!(is_opaque(solid), "{} must be opaque", name(solid));
            assert!(!is_liquid(solid), "{} must not be liquid", name(solid));
        }
    }

    #[test]
    fn original_core_colours_are_unchanged() {
        // A saved pre-landscape world must still render with its original colours.
        assert_eq!(color(MOSS), [0.29, 0.48, 0.27]);
        assert_eq!(color(SOIL), [0.35, 0.24, 0.17]);
        assert_eq!(color(STONE), [0.47, 0.51, 0.52]);
        assert_eq!(color(SAND), [0.72, 0.65, 0.46]);
        assert_eq!(color(WOOD), [0.30, 0.20, 0.13]);
        assert_eq!(color(CANOPY), [0.19, 0.38, 0.28]);
        assert_eq!(color(MINERAL), [0.42, 0.77, 0.72]);
    }

    #[test]
    fn landscape_ids_do_not_collide_with_the_detail_palette() {
        // Detail owns 10..=48 (matterweave-detail::material).
        for material in [SNOW, ICE, WATER, GRAVEL, CLAY] {
            assert!(
                !(10..=48).contains(&material),
                "{material} collides with detail"
            );
        }
    }
}
