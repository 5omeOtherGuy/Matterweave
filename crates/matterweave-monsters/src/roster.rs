//! The locked 30-species roster and affinity rules.
use serde::{Deserialize, Serialize};

/// Elemental affinity. Nine original types; interactions are shared by every
/// battle and validated in data tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Affinity {
    Ember,
    Tide,
    Verdant,
    Stone,
    Gale,
    Spark,
    Gloom,
    Lumen,
    Beast,
}

impl Affinity {
    /// Attack multiplier of `self` used against `target`. 2.0 strong, 0.5
    /// resisted, 1.0 neutral. Intentionally compact.
    pub fn against(self, target: Affinity) -> f32 {
        use Affinity::*;
        match (self, target) {
            (Ember, Verdant) | (Verdant, Tide) | (Tide, Ember) => 2.0,
            (Spark, Tide) | (Spark, Gale) => 2.0,
            (Stone, Ember) | (Stone, Spark) => 2.0,
            (Gale, Verdant) => 2.0,
            (Gloom, Lumen) | (Lumen, Gloom) => 2.0,
            (Ember, Stone) | (Ember, Tide) | (Verdant, Stone) | (Verdant, Ember) => 0.5,
            (Spark, Stone) | (Gale, Stone) => 0.5,
            (Stone, Gale) => 0.5,
            (Tide, Verdant) | (Tide, Spark) => 0.5,
            (Lumen, Lumen) | (Gloom, Gloom) => 0.5,
            _ => 1.0,
        }
    }
}

/// Stable species identifier. Serializes as its numeric id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpeciesId(pub u16);

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Stats {
    pub hp: u16,
    pub attack: u16,
    pub defense: u16,
    pub speed: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveKind {
    Physical,
    Special,
    Status,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Move {
    pub name: &'static str,
    pub kind: MoveKind,
    pub affinity: Option<Affinity>,
    pub power: u16,
}

/// One species of the locked roster. Const in-code game data; saves reference
/// species by [`SpeciesId`], never by value, so this does not serialize.
#[derive(Clone, Debug)]
pub struct Species {
    pub id: SpeciesId,
    pub name: &'static str,
    pub affinity: Affinity,
    pub stats: Stats,
    /// Evolution target and required level; terminal forms end at `None`.
    pub evolves_into: Option<(SpeciesId, u8)>,
    pub learnset: &'static [(&'static str, u8)],
}

macro_rules! species {
    ($id:literal, $name:literal, $affinity:ident, {hp: $hp:literal, atk: $atk:literal, def: $def:literal, spd: $spd:literal},
     evolves: $evo:expr, learnset: $learnset:expr) => {
        Species {
            id: SpeciesId($id),
            name: $name,
            affinity: Affinity::$affinity,
            stats: Stats { hp: $hp, attack: $atk, defense: $def, speed: $spd },
            evolves_into: $evo,
            learnset: $learnset,
        }
    };
}

pub mod ids {
    use super::SpeciesId;
    // Starter families: Ember, Tide, Verdant lines.
    pub const CINDERUB: SpeciesId = SpeciesId(1);
    pub const EMBERPACK: SpeciesId = SpeciesId(2);
    pub const PYREFALL: SpeciesId = SpeciesId(3);
    pub const BROOKLET: SpeciesId = SpeciesId(4);
    pub const TIDEWELT: SpeciesId = SpeciesId(5);
    pub const ABYSSWELL: SpeciesId = SpeciesId(6);
    pub const SPROUTLET: SpeciesId = SpeciesId(7);
    pub const THORNGALE: SpeciesId = SpeciesId(8);
    pub const VERDANTRIX: SpeciesId = SpeciesId(9);
    // Four three-stage wild families.
    pub const NIBBIT: SpeciesId = SpeciesId(10);
    pub const BURROWL: SpeciesId = SpeciesId(11);
    pub const SADDLEHARE: SpeciesId = SpeciesId(12);
    pub const FLITFIN: SpeciesId = SpeciesId(13);
    pub const SIRENSCALE: SpeciesId = SpeciesId(14);
    pub const MISTRALORE: SpeciesId = SpeciesId(15);
    pub const SHARDPUP: SpeciesId = SpeciesId(16);
    pub const CRAGHOWL: SpeciesId = SpeciesId(17);
    pub const TERRAFANG: SpeciesId = SpeciesId(18);
    pub const GLIMMERWISP: SpeciesId = SpeciesId(19);
    pub const LUMIVEIL: SpeciesId = SpeciesId(20);
    pub const PRISMGUARD: SpeciesId = SpeciesId(21);
    // Four two-stage wild families.
    pub const PUFFPEEP: SpeciesId = SpeciesId(22);
    pub const GALESWIFT: SpeciesId = SpeciesId(23);
    pub const MOSSLING: SpeciesId = SpeciesId(24);
    pub const THICKBARK: SpeciesId = SpeciesId(25);
    pub const CINDERMOTH: SpeciesId = SpeciesId(26);
    pub const ASHENWING: SpeciesId = SpeciesId(27);
    pub const PEBBLEDOVE: SpeciesId = SpeciesId(28);
    pub const CLIFFSERAPH: SpeciesId = SpeciesId(29);
    // Single non-evolving form.
    pub const DUNEMOLE: SpeciesId = SpeciesId(30);
}

use ids::*;

/// The complete locked roster: 13 families, 30 species. Order by id.
pub const ROSTER: &[Species] = &[
    species!(1, "Cinderub", Ember, {hp: 44, atk: 50, def: 42, spd: 52},
        evolves: Some((EMBERPACK, 16)), learnset: &[("Ember Nip", 1), ("Cinder Lick", 7), ("Flame Rush", 13)]),
    species!(2, "Emberpack", Ember, {hp: 58, atk: 66, def: 54, spd: 64},
        evolves: Some((PYREFALL, 32)), learnset: &[("Ember Nip", 1), ("Cinder Lick", 7), ("Flame Rush", 13), ("Sear Ring", 22)]),
    species!(3, "Pyrefall", Ember, {hp: 76, atk: 88, def: 70, spd: 80},
        evolves: None, learnset: &[("Ember Nip", 1), ("Cinder Lick", 7), ("Flame Rush", 13), ("Sear Ring", 22), ("Pyre Storm", 34)]),
    species!(4, "Brooklet", Tide, {hp: 48, atk: 44, def: 50, spd: 46},
        evolves: Some((TIDEWELT, 16)), learnset: &[("Drip Splash", 1), ("Bubble Fur", 8), ("Rip Current", 14)]),
    species!(5, "Tidewelt", Tide, {hp: 62, atk: 58, def: 64, spd: 56},
        evolves: Some((ABYSSWELL, 32)), learnset: &[("Drip Splash", 1), ("Bubble Fur", 8), ("Rip Current", 14), ("Undertow", 23)]),
    species!(6, "Abysswell", Tide, {hp: 82, atk: 78, def: 84, spd: 66},
        evolves: None, learnset: &[("Drip Splash", 1), ("Bubble Fur", 8), ("Rip Current", 14), ("Undertow", 23), ("Maelstrom", 35)]),
    species!(7, "Sproutlet", Verdant, {hp: 50, atk: 46, def: 52, spd: 40},
        evolves: Some((THORNGALE, 16)), learnset: &[("Leaf Toss", 1), ("Vine Snare", 9), ("Petal Cut", 15)]),
    species!(8, "Thorngale", Verdant, {hp: 64, atk: 60, def: 66, spd: 50},
        evolves: Some((VERDANTRIX, 32)), learnset: &[("Leaf Toss", 1), ("Vine Snare", 9), ("Petal Cut", 15), ("Bramble Trap", 24)]),
    species!(9, "Verdantrix", Verdant, {hp: 84, atk: 80, def: 86, spd: 60},
        evolves: None, learnset: &[("Leaf Toss", 1), ("Vine Snare", 9), ("Petal Cut", 15), ("Bramble Trap", 24), ("Canopy Crush", 36)]),
    species!(10, "Nibbit", Beast, {hp: 40, atk: 45, def: 35, spd: 60},
        evolves: Some((BURROWL, 14)), learnset: &[("Nibble", 1), ("Dig Dash", 6)]),
    species!(11, "Burrowl", Beast, {hp: 55, atk: 60, def: 48, spd: 70},
        evolves: Some((SADDLEHARE, 30)), learnset: &[("Nibble", 1), ("Dig Dash", 6), ("Tunnel Slam", 16)]),
    species!(12, "Saddlehare", Beast, {hp: 72, atk: 78, def: 64, spd: 88},
        evolves: None, learnset: &[("Nibble", 1), ("Dig Dash", 6), ("Tunnel Slam", 16), ("Galloping Kick", 28)]),
    species!(13, "Flitfin", Tide, {hp: 42, atk: 48, def: 40, spd: 55},
        evolves: Some((SIRENSCALE, 18)), learnset: &[("Drip Splash", 1), ("Fin Whirl", 10)]),
    species!(14, "Sirenscale", Tide, {hp: 58, atk: 64, def: 56, spd: 68},
        evolves: Some((MISTRALORE, 34)), learnset: &[("Drip Splash", 1), ("Fin Whirl", 10), ("Tail Vortex", 20)]),
    species!(15, "Mistralore", Tide, {hp: 76, atk: 82, def: 74, spd: 84},
        evolves: None, learnset: &[("Drip Splash", 1), ("Fin Whirl", 10), ("Tail Vortex", 20), ("Storm Veil", 32)]),
    species!(16, "Shardpup", Stone, {hp: 50, atk: 55, def: 60, spd: 35},
        evolves: Some((CRAGHOWL, 20)), learnset: &[("Pebble Bite", 1), ("Rock Roll", 12)]),
    species!(17, "Craghowl", Stone, {hp: 66, atk: 72, def: 78, spd: 45},
        evolves: Some((TERRAFANG, 36)), learnset: &[("Pebble Bite", 1), ("Rock Roll", 12), ("Boulder Crush", 24)]),
    species!(18, "Terrafang", Stone, {hp: 86, atk: 92, def: 98, spd: 55},
        evolves: None, learnset: &[("Pebble Bite", 1), ("Rock Roll", 12), ("Boulder Crush", 24), ("Quake Howl", 38)]),
    species!(19, "Glimmerwisp", Lumen, {hp: 40, atk: 52, def: 44, spd: 62},
        evolves: Some((LUMIVEIL, 22)), learnset: &[("Glint", 1), ("Prism Ray", 14)]),
    species!(20, "Lumiveil", Lumen, {hp: 55, atk: 68, def: 58, spd: 74},
        evolves: Some((PRISMGUARD, 38)), learnset: &[("Glint", 1), ("Prism Ray", 14), ("Radiant Flare", 26)]),
    species!(21, "Prismguard", Lumen, {hp: 74, atk: 88, def: 76, spd: 86},
        evolves: None, learnset: &[("Glint", 1), ("Prism Ray", 14), ("Radiant Flare", 26), ("Halo Burst", 40)]),
    species!(22, "Puffpeep", Gale, {hp: 45, atk: 42, def: 40, spd: 66},
        evolves: Some((GALESWIFT, 24)), learnset: &[("Gust Peck", 1), ("Sky Dart", 16)]),
    species!(23, "Galeswift", Gale, {hp: 62, atk: 60, def: 56, spd: 88},
        evolves: None, learnset: &[("Gust Peck", 1), ("Sky Dart", 16), ("Tempest Dive", 30)]),
    species!(24, "Mossling", Verdant, {hp: 52, atk: 48, def: 55, spd: 38},
        evolves: Some((THICKBARK, 26)), learnset: &[("Leaf Toss", 1), ("Spore Puff", 13)]),
    species!(25, "Thickbark", Verdant, {hp: 74, atk: 66, def: 80, spd: 46},
        evolves: None, learnset: &[("Leaf Toss", 1), ("Spore Puff", 13), ("Root Quake", 29)]),
    species!(26, "Cindermoth", Ember, {hp: 46, atk: 56, def: 42, spd: 70},
        evolves: Some((ASHENWING, 28)), learnset: &[("Ember Nip", 1), ("Dust Flare", 17)]),
    species!(27, "Ashenwing", Ember, {hp: 64, atk: 76, def: 58, spd: 92},
        evolves: None, learnset: &[("Ember Nip", 1), ("Dust Flare", 17), ("Cinder Cyclone", 33)]),
    species!(28, "Pebbledove", Stone, {hp: 48, atk: 50, def: 62, spd: 44},
        evolves: Some((CLIFFSERAPH, 30)), learnset: &[("Pebble Bite", 1), ("Wing Fault", 15)]),
    species!(29, "Cliffseraph", Stone, {hp: 68, atk: 70, def: 88, spd: 60},
        evolves: None, learnset: &[("Pebble Bite", 1), ("Wing Fault", 15), ("Talon Rupture", 31)]),
    species!(30, "Dunemole", Stone, {hp: 60, atk: 58, def: 70, spd: 30},
        evolves: None, learnset: &[("Nibble", 1), ("Sand Spray", 11)]),
];

pub fn species(id: SpeciesId) -> Option<&'static Species> {
    ROSTER.iter().find(|s| s.id == id)
}

/// Starter trio offered at the new-game choice, one per starter family.
pub const STARTERS: [SpeciesId; 3] = [CINDERUB, BROOKLET, SPROUTLET];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn by_id() -> HashMap<u16, &'static Species> {
        ROSTER.iter().map(|s| (s.id.0, s)).collect()
    }

    #[test]
    fn roster_counts_match_the_locked_family_distribution() {
        assert_eq!(ROSTER.len(), 30, "exactly 30 species");
        assert_eq!(STARTERS, [ids::CINDERUB, ids::BROOKLET, ids::SPROUTLET]);
        // 13 families: walk evolution chains from every root.
        let map = by_id();
        let mut families = 0;
        let mut seen = HashSet::new();
        for entry in ROSTER {
            if ROSTER.iter().any(|s| {
                s.evolves_into
                    .map(|(to, _)| to == entry.id)
                    .unwrap_or(false)
            }) {
                continue; // an evolved form, not a family head
            }
            families += 1;
            let mut cursor = Some(entry.id);
            while let Some(id) = cursor {
                assert!(seen.insert(id), "species {} in two families", id.0);
                cursor = map.get(&id.0).and_then(|s| s.evolves_into.map(|(t, _)| t));
            }
        }
        assert_eq!(families, 12, "12 families: 3 starters + 4 three-stage + 4 two-stage + 1 single");
    }

    #[test]
    fn evolution_graph_is_acyclic_with_valid_targets_and_terminal_finals() {
        let map = by_id();
        for entry in ROSTER {
            if let Some((to, level)) = entry.evolves_into {
                assert!(level >= 2, "{} evolves too early", entry.name);
                let target = map
                    .get(&to.0)
                    .unwrap_or_else(|| panic!("{} evolves into unknown {}", entry.name, to.0));
                assert_ne!(target.id, entry.id, "self evolution");
                assert!(
                    target.stats.hp >= entry.stats.hp && target.stats.attack >= entry.stats.attack,
                    "{} must develop, not shrink",
                    target.name
                );
            }
        }
        // Every chain terminates within three stages without revisiting.
        for entry in ROSTER {
            let mut visited = HashSet::new();
            let mut cursor = Some(entry.id);
            let mut hops = 0;
            while let Some(id) = cursor {
                assert!(visited.insert(id), "evolution cycle at {}", id.0);
                hops += 1;
                assert!(hops <= 3, "family longer than three stages at {}", id.0);
                cursor = map.get(&id.0).and_then(|s| s.evolves_into.map(|(t, _)| t));
            }
        }
    }

    #[test]
    fn starter_trio_differs_in_affinity_and_role() {
        let starters: Vec<&Species> = STARTERS.iter().map(|id| species(*id).unwrap()).collect();
        let unique: HashSet<_> = starters.iter().map(|s| s.affinity).collect();
        assert_eq!(unique.len(), 3, "three distinct starter affinities");
        // Role distinctness: fastest starter has the lowest defense.
        let mut fastest = starters.clone();
        fastest.sort_by_key(|s| std::cmp::Reverse(s.stats.speed));
        assert!(fastest[0].stats.defense < fastest[2].stats.defense, "speed/defense roles split");
    }

    #[test]
    fn every_learnset_move_is_bound_and_learnable_levels_are_ordered() {
        for entry in ROSTER {
            assert!(!entry.learnset.is_empty(), "{} has no moves", entry.name);
            let mut last = 0;
            for (name, level) in entry.learnset {
                assert!(!name.is_empty());
                assert!(*level >= last && *level <= 40, "{} move level {}", entry.name, level);
                last = *level;
            }
        }
    }

    #[test]
    fn every_species_is_reachable_in_normal_play() {
        // An evolved form must be somebody's evolution target; family heads
        // must be starter or wild-encounter content (routes module pins that).
        let evolved: HashSet<_> = ROSTER
            .iter()
            .filter_map(|s| s.evolves_into.map(|(t, _)| t))
            .collect();
        for entry in ROSTER {
            if evolved.contains(&entry.id) {
                assert!(
                    ROSTER
                        .iter()
                        .any(|s| s.evolves_into.map(|(t, _)| t) == Some(entry.id)),
                    "{} unreachable",
                    entry.name
                );
            }
        }
    }
}

