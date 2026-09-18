//! Shared move table. Every learnset name must resolve here; damage reads the
//! move's power from this table, never from the learn level.
use crate::roster::MoveKind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveData {
    pub name: &'static str,
    pub kind: MoveKind,
    pub power: u16,
}

macro_rules! move_data {
    ($name:literal, $kind:ident, $power:literal) => {
        MoveData {
            name: $name,
            kind: MoveKind::$kind,
            power: $power,
        }
    };
}

pub const MOVES: &[MoveData] = &[
    move_data!("Ember Nip", Physical, 35),
    move_data!("Cinder Lick", Special, 40),
    move_data!("Flame Rush", Physical, 55),
    move_data!("Sear Ring", Special, 60),
    move_data!("Pyre Storm", Special, 85),
    move_data!("Drip Splash", Special, 35),
    move_data!("Bubble Fur", Special, 40),
    move_data!("Rip Current", Physical, 55),
    move_data!("Undertow", Special, 60),
    move_data!("Maelstrom", Special, 85),
    move_data!("Leaf Toss", Physical, 35),
    move_data!("Vine Snare", Status, 0),
    move_data!("Petal Cut", Physical, 55),
    move_data!("Bramble Trap", Status, 0),
    move_data!("Canopy Crush", Physical, 85),
    move_data!("Nibble", Physical, 35),
    move_data!("Dig Dash", Physical, 40),
    move_data!("Tunnel Slam", Physical, 55),
    move_data!("Galloping Kick", Physical, 70),
    move_data!("Fin Whirl", Physical, 45),
    move_data!("Tail Vortex", Physical, 60),
    move_data!("Storm Veil", Status, 0),
    move_data!("Pebble Bite", Physical, 35),
    move_data!("Rock Roll", Physical, 45),
    move_data!("Boulder Crush", Physical, 60),
    move_data!("Quake Howl", Physical, 80),
    move_data!("Glint", Special, 35),
    move_data!("Prism Ray", Special, 50),
    move_data!("Radiant Flare", Special, 65),
    move_data!("Halo Burst", Special, 85),
    move_data!("Gust Peck", Physical, 35),
    move_data!("Sky Dart", Physical, 45),
    move_data!("Tempest Dive", Physical, 70),
    move_data!("Spore Puff", Status, 0),
    move_data!("Root Quake", Physical, 65),
    move_data!("Dust Flare", Special, 50),
    move_data!("Cinder Cyclone", Special, 75),
    move_data!("Wing Fault", Physical, 45),
    move_data!("Talon Rupture", Physical, 65),
    move_data!("Sand Spray", Special, 40),
];

pub fn move_data(name: &str) -> Option<&'static MoveData> {
    MOVES.iter().find(|m| m.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roster::ROSTER;
    use std::collections::HashSet;

    #[test]
    fn move_names_are_unique_and_powered_except_status() {
        let mut seen = HashSet::new();
        for m in MOVES {
            assert!(seen.insert(m.name), "duplicate move {}", m.name);
            if m.kind == MoveKind::Status {
                assert_eq!(m.power, 0, "status {} must not deal power", m.name);
            } else {
                assert!((25..=90).contains(&m.power), "{} power out of band", m.name);
            }
        }
    }

    #[test]
    fn every_learnset_move_resolves_in_the_table() {
        for species in ROSTER {
            for (name, _) in species.learnset {
                assert!(
                    move_data(name).is_some(),
                    "{} learns unknown move {name}",
                    species.name
                );
            }
        }
    }
}
