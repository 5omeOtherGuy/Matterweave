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

/// How much a material's surface colour varies from cell to cell.
///
/// Both amplitudes are in 1/256ths of the base colour and apply to deltas in
/// `-128..=127` (see [`tone`]), so an amplitude of 32 is a peak of 1/8 of the
/// colour and never reached in practice: two independent uniform terms are
/// averaged into the luminance channel.
///
/// * `luma` scales one correlated term shared by all three channels. It moves
///   value without moving hue, which is what rock and snow want: a stone face
///   is lit stone, not a colour wheel.
/// * `chroma` scales the per-channel remainder. It moves hue and saturation
///   together, which is what a grass sward wants - blades change colour more
///   than they change brightness.
///
/// [WATER] is zero in both. Its near surface is a derived mesh whose vertex
/// colour encodes depth and whose shader owns the palette, and the coarse ring
/// water is recognised by its exact palette colour, so tinting the mesh would
/// turn the far sea into terrain. Water varies through its own pass instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToneFamily {
    pub luma: i32,
    pub chroma: i32,
}

/// Tone family of a material: the magnitudes are the measured ones (see the
/// module tests and the PR): enough to break a flat field, small enough that a
/// surface still reads as its material, and separated so families read
/// differently rather than as one global noise.
pub fn tone_family(material: u8) -> ToneFamily {
    match material {
        // Grass and leaves: colour variation first, brightness second.
        MOSS | CANOPY => ToneFamily {
            luma: 34,
            chroma: 74,
        },
        // Rock: brightness first, hue only as drift.
        STONE => ToneFamily {
            luma: 100,
            chroma: 32,
        },
        GRAVEL => ToneFamily {
            luma: 84,
            chroma: 36,
        },
        MINERAL => ToneFamily {
            luma: 64,
            chroma: 72,
        },
        // Sand is a sorted, wind-blown surface: the flattest family here.
        SAND => ToneFamily {
            luma: 28,
            chroma: 20,
        },
        SOIL => ToneFamily {
            luma: 72,
            chroma: 54,
        },
        CLAY => ToneFamily {
            luma: 62,
            chroma: 40,
        },
        WOOD => ToneFamily {
            luma: 62,
            chroma: 50,
        },
        SNOW => ToneFamily {
            luma: 30,
            chroma: 18,
        },
        ICE => ToneFamily {
            luma: 40,
            chroma: 30,
        },
        // Air is never drawn, and water varies elsewhere (see [`ToneFamily`]).
        _ => ToneFamily { luma: 0, chroma: 0 },
    }
}

/// Salt separating the tone hash from every generator field. A constant, not a
/// per-world seed: the same cell keeps the same tone in every world, so an
/// edit or a reload never re-rolls the look of terrain that did not change.
const TONE_SALT: u64 = 0xd1b5_4a32_d192_ed03;

/// Deterministic per-voxel tone deltas for one integer world cell, in 1/256ths
/// of the material colour, one per channel.
///
/// Integer throughout: the hash, the two terms and the scaling are all integer
/// arithmetic, so ARM64 and x86-64 produce the same deltas for the same cell,
/// and a mesh built on either target carries the same colour. The cell is the
/// authority, never the draw: moving the camera cannot change a surface's tone.
pub fn tone(material: u8, x: i32, y: i32, z: i32) -> [i32; 3] {
    let family = tone_family(material);
    if family.luma == 0 && family.chroma == 0 {
        return [0; 3];
    }
    // Material in the seed: two materials sharing a cell must not share a tone.
    let hash = crate::landscape::hash4(TONE_SALT ^ u64::from(material), x, y, z);
    let d = [
        (hash & 0xff) as i32 - 128,
        ((hash >> 8) & 0xff) as i32 - 128,
        ((hash >> 16) & 0xff) as i32 - 128,
    ];
    let luma = (d[0] + d[1] + d[2]) / 3;
    std::array::from_fn(|channel| (family.luma * luma + family.chroma * (d[channel] - luma)) / 128)
}

/// Mean of `sum`, a sum of [`tone`] deltas over `cells` cells, in the same
/// 1/256th units. A merged face uses this to carry one colour that stands for
/// every cell it covers instead of the colour of one sampled cell.
/// Truncating integer division, so the same cells give the same mean anywhere.
pub fn mean_tone(sum: [i32; 3], cells: i32) -> [i32; 3] {
    let cells = cells.max(1);
    sum.map(|value| value / cells)
}

/// Apply a tone to a base colour: one multiply-add per channel, with a power of
/// two divisor so the result rounds identically on every target.
pub fn tint(base: [f32; 3], tone: [i32; 3]) -> [f32; 3] {
    std::array::from_fn(|channel| base[channel] * (1.0 + tone[channel] as f32 * (1.0 / 256.0)))
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
    fn tone_is_bounded_by_its_material_family_and_deterministic() {
        for material in PALETTE {
            let family = tone_family(material);
            // Amplitudes are in 1/256ths and none may exceed 128: beyond that
            // a peak chroma term could swing a channel past double its colour.
            assert!(
                (0..=128).contains(&family.luma) && (0..=128).contains(&family.chroma),
                "{} has an untuned tone family {family:?}",
                name(material)
            );
            assert_eq!(
                tone(material, 31, -7, 500),
                tone(material, 31, -7, 500),
                "{} tone must be a pure function of the cell",
                name(material)
            );
            for (x, y, z) in [
                (0, 0, 0),
                (1, 2, 3),
                (-91, 60, 7000),
                (i32::MIN, 0, i32::MAX),
            ] {
                let deltas = tone(material, x, y, z);
                // The luminance term is one average of three bounded bytes and
                // the chroma term is a byte minus that average, so the sum of
                // the two amplitudes is not the bound: the chroma term alone
                // can reach 255.
                let bound = (family.luma * 128 + family.chroma * 255) / 128;
                for delta in deltas {
                    assert!(
                        (-bound..=bound).contains(&delta),
                        "{} tone {delta} at ({x}, {y}, {z}) exceeds {bound}",
                        name(material)
                    );
                }
            }
        }
        assert_eq!(tone(WATER, 3, 4, 5), [0; 3], "water varies in its own pass");
        assert_eq!(tone(AIR, 3, 4, 5), [0; 3], "air is never drawn");
    }

    #[test]
    fn tone_is_material_specific_and_the_families_differ() {
        // Two materials at the same cell must not share a tone.
        assert_ne!(tone(STONE, 8, 9, 10), tone(MOSS, 8, 9, 10));
        // The families are ordered the way the spec describes them: grass
        // varies more in colour than in value, rock the other way round, and
        // sand varies least of the three ground families.
        let grass = tone_family(MOSS);
        let rock = tone_family(STONE);
        let sand = tone_family(SAND);
        assert!(grass.chroma > grass.luma, "grass: hue/saturation first");
        assert!(rock.luma > rock.chroma, "rock: luminance first");
        assert!(
            sand.luma + sand.chroma < grass.luma + grass.chroma
                && sand.luma + sand.chroma < rock.luma + rock.chroma,
            "sand varies least"
        );
        // A family with no amplitude returns the base colour exactly, so a
        // saved world that predates this module still matches its old mesh.
        assert_eq!(tint(color(SAND), [0; 3]), color(SAND));
    }

    #[test]
    fn tone_aggregates_over_a_run_without_bias() {
        // The mean of a run is the mean of its deltas, in integer arithmetic.
        let a = [10, -4, 6];
        let b = [-2, 12, -8];
        let sum = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        assert_eq!(mean_tone(sum, 2), [4, 4, -1]);
        // An empty run cannot happen, and never divides by zero.
        assert_eq!(mean_tone(sum, 0), sum);
        // A run long enough to average a white-noise family back to zero keeps
        // the base colour exactly.
        assert_eq!(
            tint([0.5, 0.25, 0.75], mean_tone([0; 3], 1024)),
            [0.5, 0.25, 0.75]
        );
    }

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
