//! Turn-based battle resolution. Deterministic given a seeded roll source so
//! tests and replays are stable; the UI only renders the event log.

use crate::monsters::{damage, GameError, Monster};
use crate::state::{capture_chance, Game};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattleAction {
    Fight(usize),
    Capture,
    Heal(usize),
    Switch(usize),
    Escape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BattleEvent {
    PlayerUsedMove {
        name: String,
        damage: u16,
    },
    WildUsedMove {
        name: String,
        damage: u16,
    },
    PlayerFainted,
    WildFainted,
    CaptureSucceeded {
        party_index: Option<usize>,
    },
    CaptureFailed,
    Escaped,
    SwitchIn {
        index: usize,
    },
    Heal {
        index: usize,
        restored: u16,
    },
    LevelUp {
        index: usize,
        level: u8,
        learned: Option<String>,
    },
    Evolved {
        index: usize,
        into: u16,
    },
    NoEffect {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattlePhase {
    PlayerChoice,
    Won,
    Lost,
    Captured,
    Fled,
}

/// Deterministic roll source: a tiny xorshift so battles replay exactly and
/// never depend on wall time.
#[derive(Clone, Debug)]
pub struct Rolls(u64);

impl Rolls {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }

    pub fn percent(&mut self) -> u32 {
        self.next() % 100
    }
}

pub struct Battle {
    pub wild: Monster,
    pub phase: BattlePhase,
    pub log: Vec<BattleEvent>,
    xp_award: u32,
}

impl Battle {
    pub fn wild_encounter(wild: Monster, player_level: u8) -> Self {
        let xp_award = 8 + wild.level as u32 * 4 + player_level as u32 / 2;
        Battle {
            wild,
            phase: BattlePhase::PlayerChoice,
            log: Vec::new(),
            xp_award,
        }
    }

    /// Total XP this battle is worth when won.
    pub fn xp_reward(&self) -> u32 {
        self.xp_award
    }
}

/// Resolve one player action. `game` owns the party; the battle owns the wild.
pub fn resolve(
    game: &mut Game,
    battle: &mut Battle,
    active: usize,
    action: BattleAction,
    rolls: &mut Rolls,
) -> Result<(), GameError> {
    if battle.phase != BattlePhase::PlayerChoice {
        return Err(GameError::InvalidTransition);
    }
    match action {
        BattleAction::Escape => {
            let player = &game.party[active];
            let player_speed = player.data().stats.speed as u32;
            let wild_speed = battle.wild.data().stats.speed as u32;
            let chance = (40 + player_speed.saturating_sub(wild_speed) / 2).min(95);
            if rolls.percent() < chance {
                battle.phase = BattlePhase::Fled;
                battle.log.push(BattleEvent::Escaped);
                return Ok(());
            }
            battle.log.push(BattleEvent::NoEffect {
                reason: "The wild creature blocks the way.".into(),
            });
            wild_strike(game, battle, active, rolls);
        }
        BattleAction::Capture => {
            let chance = capture_chance(&battle.wild);
            if rolls.percent() < chance {
                match game.capture(&battle.wild) {
                    Ok(_) => {
                        battle.phase = BattlePhase::Captured;
                        battle
                            .log
                            .push(BattleEvent::CaptureSucceeded { party_index: None });
                    }
                    Err(_) => {
                        battle.log.push(BattleEvent::CaptureFailed);
                        wild_strike(game, battle, active, rolls);
                    }
                }
            } else {
                battle.log.push(BattleEvent::CaptureFailed);
                wild_strike(game, battle, active, rolls);
            }
        }
        BattleAction::Heal(index) => match game.heal_member(index) {
            Ok(restored) => battle.log.push(BattleEvent::Heal { index, restored }),
            Err(_) => {
                battle.log.push(BattleEvent::NoEffect {
                    reason: "No tonic to use.".into(),
                });
                wild_strike(game, battle, active, rolls);
            }
        },
        BattleAction::Switch(index) => {
            if index >= game.party.len() {
                return Err(GameError::InvalidTransition);
            }
            battle.log.push(BattleEvent::SwitchIn { index });
            wild_strike(game, battle, index, rolls);
        }
        BattleAction::Fight(move_index) => {
            let player_move = game.party[active]
                .moves
                .get(move_index)
                .cloned()
                .ok_or(GameError::NoMoves)?;
            let player_first =
                game.party[active].data().stats.speed >= battle.wild.data().stats.speed;
            if player_first {
                player_attack(game, battle, active, &player_move.name, rolls)?;
                if battle.phase == BattlePhase::PlayerChoice {
                    wild_strike(game, battle, active, rolls);
                }
            } else {
                wild_strike(game, battle, active, rolls);
                if battle.phase == BattlePhase::PlayerChoice {
                    player_attack(game, battle, active, &player_move.name, rolls)?;
                }
            }
        }
    }
    Ok(())
}

fn player_attack(
    game: &mut Game,
    battle: &mut Battle,
    active: usize,
    move_name: &str,
    rolls: &mut Rolls,
) -> Result<(), GameError> {
    let roll = rolls.next();
    let dealt = damage(&game.party[active], &battle.wild, move_name, roll)?;
    battle.wild.current_hp = battle.wild.current_hp.saturating_sub(dealt);
    if let Some(slot) = game.party[active]
        .moves
        .iter_mut()
        .find(|slot| slot.name == move_name)
    {
        slot.current_pp = slot.current_pp.saturating_sub(1);
    }
    battle.log.push(BattleEvent::PlayerUsedMove {
        name: move_name.to_string(),
        damage: dealt,
    });
    if battle.wild.current_hp == 0 {
        battle.log.push(BattleEvent::WildFainted);
        let before = game.party[active].level;
        let (level, learned) = game.grant_xp(active, battle.xp_award)?;
        if level != before || learned.is_some() {
            battle.log.push(BattleEvent::LevelUp {
                index: active,
                level,
                learned,
            });
        }
        // Evolution is offered when the gate opens; applied immediately so a
        // won battle cannot be lost to a forgotten menu.
        if game.party[active]
            .data()
            .evolves_into
            .is_some_and(|(_, need)| level >= need)
        {
            if let Ok(into) = game.evolve_check(active) {
                battle.log.push(BattleEvent::Evolved {
                    index: active,
                    into: into.0,
                });
            }
        }
        battle.phase = BattlePhase::Won;
    }
    Ok(())
}

fn wild_strike(game: &mut Game, battle: &mut Battle, active: usize, rolls: &mut Rolls) {
    if battle.wild.current_hp == 0 || active >= game.party.len() {
        return;
    }
    let chosen = pick_wild_move(&battle.wild, rolls);
    let Ok(move_name) = chosen else { return };
    let roll = rolls.next();
    let dealt = damage(&battle.wild, &game.party[active], &move_name, roll).unwrap_or(0);
    let member = &mut game.party[active];
    member.current_hp = member.current_hp.saturating_sub(dealt);
    battle.log.push(BattleEvent::WildUsedMove {
        name: move_name,
        damage: dealt,
    });
    if member.current_hp == 0 {
        battle.log.push(BattleEvent::PlayerFainted);
        battle.phase = BattlePhase::Lost;
    }
}

fn pick_wild_move(wild: &Monster, rolls: &mut Rolls) -> Result<String, GameError> {
    let usable: Vec<&str> = wild
        .moves
        .iter()
        .filter(|slot| {
            slot.current_pp > 0 && crate::moves::move_data(&slot.name).is_some_and(|m| m.power > 0)
        })
        .map(|slot| slot.name.as_str())
        .collect();
    if usable.is_empty() {
        return Err(GameError::NoMoves);
    }
    Ok(usable[rolls.next() as usize % usable.len()].to_string())
}
