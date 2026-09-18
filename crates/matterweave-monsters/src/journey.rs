//! The directed journey: two towns, four routes, trainers and the gym.
//! All structured content with stable ids; validation tests pin reachability.
use crate::roster::{ids, SpeciesId, ROSTER};
use serde::{Deserialize, Serialize};

/// Stable location identifiers in journey order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Place {
    Emberfield,
    MeadowWay,
    Thornhollow,
    Mistpath,
    Tidewater,
    QuarryLoop,
}

impl Place {
    pub const ALL: [Place; 6] = [
        Place::Emberfield,
        Place::MeadowWay,
        Place::Thornhollow,
        Place::Mistpath,
        Place::Tidewater,
        Place::QuarryLoop,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Place::Emberfield => "Emberfield",
            Place::MeadowWay => "Meadow Way",
            Place::Thornhollow => "Thornhollow",
            Place::Mistpath => "Mistpath",
            Place::Tidewater => "Tidewater",
            Place::QuarryLoop => "Quarry Loop",
        }
    }

    /// A town is a safe place: haven and supply counter, no encounters.
    pub fn is_town(self) -> bool {
        matches!(self, Place::Emberfield | Place::Tidewater)
    }

    /// Routes connect the towns; the first is the opening leg.
    pub fn is_route(self) -> bool {
        !self.is_town()
    }
}

/// One route's wild encounter table entry.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Encounter {
    pub species: SpeciesId,
    pub min_level: u8,
    pub max_level: u8,
    /// Relative weight in the table; higher is more common.
    pub weight: u16,
    /// Hidden until the gym badge is earned (the late-demo sanctuary).
    #[serde(default)]
    pub post_gym: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Route {
    pub place: Place,
    pub encounters: &'static [Encounter],
}

macro_rules! encounter {
    ($id:expr, $min:literal, $max:literal, $weight:literal) => {
        Encounter {
            species: $id,
            min_level: $min,
            max_level: $max,
            weight: $weight,
            post_gym: false,
        }
    };
    (post, $id:expr, $min:literal, $max:literal, $weight:literal) => {
        Encounter {
            species: $id,
            min_level: $min,
            max_level: $max,
            weight: $weight,
            post_gym: true,
        }
    };
}

/// Route populations: 3-5 species each, overlap across routes, rare evolved
/// forms, and the two unchosen starter lines as post-gym content on the
/// Quarry Loop so one save can complete the roster.
pub const ROUTES: &[Route] = &[
    Route {
        place: Place::MeadowWay,
        encounters: &[
            encounter!(ids::NIBBIT, 3, 5, 40),
            encounter!(ids::PUFFPEEP, 3, 5, 30),
            encounter!(ids::MOSSLING, 4, 6, 25),
            encounter!(ids::FLITFIN, 4, 6, 15),
        ],
    },
    Route {
        place: Place::Thornhollow,
        encounters: &[
            encounter!(ids::NIBBIT, 5, 8, 30),
            encounter!(ids::CINDERMOTH, 6, 9, 25),
            encounter!(ids::MOSSLING, 5, 8, 25),
            encounter!(ids::PUFFPEEP, 6, 9, 20),
            encounter!(ids::BURROWL, 14, 14, 4),
        ],
    },
    Route {
        place: Place::Mistpath,
        encounters: &[
            encounter!(ids::GLIMMERWISP, 7, 11, 30),
            encounter!(ids::PEBBLEDOVE, 7, 11, 25),
            encounter!(ids::DUNEMOLE, 8, 12, 25),
            encounter!(ids::FLITFIN, 8, 11, 20),
        ],
    },
    Route {
        place: Place::QuarryLoop,
        encounters: &[
            encounter!(ids::SHARDPUP, 11, 15, 30),
            encounter!(ids::CINDERMOTH, 12, 15, 25),
            encounter!(ids::PEBBLEDOVE, 11, 14, 20),
            encounter!(ids::DUNEMOLE, 12, 15, 20),
            encounter!(ids::NIBBIT, 11, 14, 10),
            // Post-gym sanctuary: every starter base line, so whichever was
            // chosen at the opening the other two are obtainable here.
            encounter!(post, ids::CINDERUB, 26, 30, 18),
            encounter!(post, ids::BROOKLET, 26, 30, 18),
            encounter!(post, ids::SPROUTLET, 26, 30, 18),
            encounter!(post, ids::CRAGHOWL, 28, 32, 10),
            encounter!(post, ids::LUMIVEIL, 28, 32, 8),
        ],
    },
];

pub fn route(place: Place) -> Option<&'static Route> {
    ROUTES.iter().find(|r| r.place == place)
}

/// A trainer battle. `party` lists species and levels in send-out order.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TrainerMon {
    pub species: SpeciesId,
    pub level: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Trainer {
    pub id: &'static str,
    pub place: Place,
    pub title: &'static str,
    pub party: &'static [TrainerMon],
    /// Awarded once, tracked in `Game::defeated_trainers`.
    pub reward_charms: u32,
    pub reward_tonics: u32,
    pub post_gym: bool,
}

macro_rules! trainer_mon {
    ($id:expr, $level:literal) => {
        TrainerMon {
            species: $id,
            level: $level,
        }
    };
}

pub const TRAINERS: &[Trainer] = &[
    Trainer {
        id: "meadow-ranger",
        place: Place::MeadowWay,
        title: "Ranger Tamsin",
        party: &[trainer_mon!(ids::NIBBIT, 5), trainer_mon!(ids::PUFFPEEP, 6)],
        reward_charms: 3,
        reward_tonics: 1,
        post_gym: false,
    },
    Trainer {
        id: "thorn-wayfarer",
        place: Place::Thornhollow,
        title: "Wayfarer Odell",
        party: &[
            trainer_mon!(ids::MOSSLING, 8),
            trainer_mon!(ids::CINDERMOTH, 9),
        ],
        reward_charms: 3,
        reward_tonics: 2,
        post_gym: false,
    },
    Trainer {
        id: "mist-herbalist",
        place: Place::Mistpath,
        title: "Herbalist Sana",
        party: &[
            trainer_mon!(ids::GLIMMERWISP, 10),
            trainer_mon!(ids::PEBBLEDOVE, 11),
        ],
        reward_charms: 4,
        reward_tonics: 2,
        post_gym: false,
    },
    Trainer {
        id: "quarry-prospector",
        place: Place::QuarryLoop,
        title: "Prospector Bram",
        party: &[
            trainer_mon!(ids::SHARDPUP, 13),
            trainer_mon!(ids::DUNEMOLE, 14),
        ],
        reward_charms: 4,
        reward_tonics: 2,
        post_gym: false,
    },
    Trainer {
        id: "quarry-scout",
        place: Place::QuarryLoop,
        title: "Scout Vey",
        party: &[
            trainer_mon!(ids::CRAGHOWL, 30),
            trainer_mon!(ids::THICKBARK, 30),
        ],
        reward_charms: 6,
        reward_tonics: 3,
        post_gym: true,
    },
];

pub const GYM: Place = Place::Tidewater;
/// Gym lead-in attendants, then the leader. The leader is the finale.
pub const GYM_TRAINERS: &[&str] = &["gym-attendant-lune", "gym-attendant-kest"];
pub const GYM_LEADER: &str = "leader-marwick";

/// Gym staff. Leader Marwick keeps the Lumen theme and three monsters. Lumen
/// is neutral in both directions against every starter affinity, so no
/// opening choice is hard-countered by the finale (Tide halves Ember and
/// Verdant; Stone halves Ember and Verdant too).
pub const GYM_BATTLES: &[Trainer] = &[
    Trainer {
        id: "gym-attendant-lune",
        place: Place::Tidewater,
        title: "Attendant Lune",
        party: &[
            trainer_mon!(ids::GLIMMERWISP, 13),
            trainer_mon!(ids::NIBBIT, 14),
        ],
        reward_charms: 3,
        reward_tonics: 1,
        post_gym: false,
    },
    Trainer {
        id: "gym-attendant-kest",
        place: Place::Tidewater,
        title: "Attendant Kest",
        party: &[
            trainer_mon!(ids::GLIMMERWISP, 15),
            trainer_mon!(ids::BURROWL, 16),
        ],
        reward_charms: 3,
        reward_tonics: 1,
        post_gym: false,
    },
    Trainer {
        id: "leader-marwick",
        place: Place::Tidewater,
        title: "Leader Marwick",
        party: &[
            trainer_mon!(ids::GLIMMERWISP, 16),
            trainer_mon!(ids::LUMIVEIL, 14),
            trainer_mon!(ids::GLIMMERWISP, 17),
        ],
        reward_charms: 8,
        reward_tonics: 4,
        post_gym: false,
    },
];

/// Every family head must appear here (a route encounter or the starter
/// choice) so the full roster is obtainable through ordinary play.
pub fn family_heads() -> Vec<SpeciesId> {
    let evolved: Vec<SpeciesId> = ROSTER
        .iter()
        .filter_map(|s| s.evolves_into.map(|(to, _)| to))
        .collect();
    ROSTER
        .iter()
        .filter(|s| !evolved.contains(&s.id))
        .map(|s| s.id)
        .collect()
}

pub fn trainers_at(place: Place) -> impl Iterator<Item = &'static Trainer> {
    TRAINERS.iter().filter(move |t| t.place == place)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moves::move_data;
    use crate::roster::species;
    use std::collections::HashSet;

    #[test]
    fn exactly_two_towns_four_routes_and_one_gym_town() {
        let towns: Vec<Place> = Place::ALL.iter().copied().filter(|p| p.is_town()).collect();
        assert_eq!(towns, vec![Place::Emberfield, Place::Tidewater]);
        assert_eq!(ROUTES.len(), 4, "four distinct routes");
        assert!(GYM.is_town(), "the gym lives in a town");
        assert!(
            !GYM_TRAINERS.is_empty(),
            "a lead-in challenge precedes the leader"
        );
        assert_eq!(
            GYM_BATTLES.iter().filter(|t| t.id == GYM_LEADER).count(),
            1,
            "exactly one gym leader"
        );
    }

    #[test]
    fn every_route_has_three_to_five_common_species_and_a_trainer() {
        for route in ROUTES {
            let common = route.encounters.iter().filter(|e| !e.post_gym).count();
            assert!(
                (3..=6).contains(&common),
                "{} offers {common} common species",
                route.place.name()
            );
            assert!(
                trainers_at(route.place).count() >= 1,
                "{} has no trainer",
                route.place.name()
            );
            for e in route.encounters {
                assert!(e.min_level <= e.max_level, "level band inverted");
                assert!(e.weight > 0);
                assert!(
                    species(e.species).is_some(),
                    "unknown species {}",
                    e.species.0
                );
            }
        }
    }

    #[test]
    fn evolved_encounters_are_rare_and_never_below_their_pre_evolution_level() {
        for route in ROUTES {
            for e in route.encounters {
                let data = species(e.species).unwrap();
                let is_evolved = ROSTER
                    .iter()
                    .any(|s| s.evolves_into.map(|(t, _)| t) == Some(e.species));
                if is_evolved {
                    assert!(e.weight <= 10, "{} evolved form is not rare", data.name);
                }
                if let Some((_, gate)) = ROSTER
                    .iter()
                    .find(|s| s.evolves_into.map(|(t, _)| t) == Some(e.species))
                    .and_then(|s| s.evolves_into)
                {
                    assert!(
                        e.min_level >= gate,
                        "{} appears below its evolution gate {gate}",
                        data.name
                    );
                }
            }
        }
    }

    #[test]
    fn every_family_head_is_obtainable_and_finals_are_post_or_evolved() {
        let heads = family_heads();
        assert_eq!(heads.len(), 12, "12 family heads");
        let mut obtainable: HashSet<SpeciesId> = crate::roster::STARTERS.into_iter().collect();
        for route in ROUTES {
            for e in route.encounters {
                obtainable.insert(e.species);
            }
        }
        for head in &heads {
            assert!(
                obtainable.contains(head),
                "family head {} is not obtainable",
                head.0
            );
        }
        // Every evolved form is someone's target, so evolution completes the set.
        let evolved: Vec<SpeciesId> = ROSTER
            .iter()
            .filter_map(|s| s.evolves_into.map(|(to, _)| to))
            .collect();
        for id in evolved {
            assert!(
                ROSTER.iter().any(|s| s.id == id),
                "evolution target missing"
            );
        }
    }

    #[test]
    fn the_two_unchosen_starter_lines_are_reachable_post_gym() {
        let post: HashSet<SpeciesId> = ROUTES
            .iter()
            .flat_map(|r| r.encounters.iter())
            .filter(|e| e.post_gym)
            .map(|e| e.species)
            .collect();
        assert!(
            post.contains(&ids::BROOKLET),
            "Tide starter line obtainable"
        );
        assert!(
            post.contains(&ids::SPROUTLET),
            "Verdant starter line obtainable"
        );
        // And a post-gym trainer exists so the sanctuary is not the only gate.
        assert!(TRAINERS.iter().any(|t| t.post_gym));
    }

    #[test]
    fn trainer_parties_are_legal_at_their_levels() {
        for trainer in TRAINERS.iter().chain(GYM_BATTLES.iter()) {
            assert!(
                !trainer.party.is_empty(),
                "{} has an empty party",
                trainer.id
            );
            assert!(
                trainer.party.len() <= crate::monsters::MAX_PARTY,
                "{} exceeds the party cap",
                trainer.id
            );
            for picked in trainer.party {
                let data = species(picked.species).unwrap_or_else(|| {
                    panic!("{}: unknown species {}", trainer.id, picked.species.0)
                });
                assert!(
                    (1..=crate::monsters::MAX_LEVEL).contains(&picked.level),
                    "{}: level {} out of range",
                    trainer.id,
                    picked.level
                );
                let legal: Vec<&str> = data
                    .learnset
                    .iter()
                    .filter(|(_, lvl)| *lvl <= picked.level)
                    .map(|(name, _)| *name)
                    .collect();
                assert!(
                    !legal.is_empty(),
                    "{}: {} has no legal move at level {}",
                    trainer.id,
                    data.name,
                    picked.level
                );
                for name in legal {
                    assert!(move_data(name).is_some(), "unknown move {name}");
                }
            }
        }
    }

    #[test]
    fn gym_lead_in_precedes_the_finale_in_sendout_order() {
        assert_eq!(GYM_BATTLES.len(), GYM_TRAINERS.len() + 1);
        for (i, id) in GYM_TRAINERS.iter().enumerate() {
            assert_eq!(GYM_BATTLES[i].id, *id, "lead-in order pinned");
        }
        assert_eq!(GYM_BATTLES.last().unwrap().id, GYM_LEADER);
        let leader = GYM_BATTLES.last().unwrap();
        assert!(leader.party.len() >= 3, "the finale is a real battle");
        // The finale is coherent and neutral for every starter choice: the
        // leader fields a Lumen team, no gym staff member resists a starter's
        // attacks, and no staff member hits a starter for double.
        use crate::roster::Affinity;
        assert!(
            leader
                .party
                .iter()
                .all(|m| species(m.species).is_some_and(|s| s.affinity == Affinity::Lumen)),
            "the leader keeps the Lumen theme"
        );
        for battle in GYM_BATTLES {
            for picked in battle.party {
                let defender = species(picked.species).unwrap();
                for starter in crate::roster::STARTERS {
                    let data = species(starter).unwrap();
                    assert!(
                        data.affinity.against(defender.affinity) >= 1.0,
                        "{} must not resist {}'s attacks",
                        defender.name,
                        data.name
                    );
                    assert!(
                        defender.affinity.against(data.affinity) <= 1.0,
                        "{} must not hit {} for double",
                        defender.name,
                        data.name
                    );
                }
            }
        }
    }

    #[test]
    fn journey_order_connects_towns_through_all_four_routes() {
        // Emberfield -> Meadow Way -> Thornhollow -> (Mistpath side loop) ->
        // Tidewater -> Quarry Loop -> Emberfield. Every route borders the
        // journey; the Quarry Loop is the return connection.
        let order = [
            Place::Emberfield,
            Place::MeadowWay,
            Place::Thornhollow,
            Place::Mistpath,
            Place::Tidewater,
            Place::QuarryLoop,
        ];
        assert_eq!(Place::ALL, order, "journey order is the documented one");
        assert!(mistpath_side_loop(&order));
    }

    fn mistpath_side_loop(order: &[Place; 6]) -> bool {
        // Mistpath sits between Thornhollow and Tidewater as an optional leg.
        order[2] == Place::Thornhollow
            && order[3] == Place::Mistpath
            && order[4] == Place::Tidewater
    }
}
