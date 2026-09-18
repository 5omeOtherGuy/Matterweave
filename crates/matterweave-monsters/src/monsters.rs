//! Monster instances, stat growth and the shared battle/capture rules.
use crate::roster::{species, SpeciesId, Stats};
use serde::{Deserialize, Serialize};

pub const MAX_PARTY: usize = 6;
pub const MOVES_PER_MONSTER: usize = 4;
pub const MAX_STORAGE: usize = 30;
pub const MAX_LEVEL: u8 = 40;
pub const PP_PER_MOVE: u8 = 20;

/// One owned monster instance. Saves reference species by id.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Monster {
    pub species: SpeciesId,
    pub nickname: Option<String>,
    pub level: u8,
    pub experience: u32,
    pub current_hp: u16,
    pub moves: Vec<MoveSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveSlot {
    pub name: String,
    pub current_pp: u8,
    pub max_pp: u8,
}

/// Errors ordinary play can hit; the UI maps each to a readable message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameError {
    PartyFull,
    StorageFull,
    InvalidSpecies(SpeciesId),
    NoMoves,
    InvalidTransition,
}

impl std::fmt::Display for GameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GameError::PartyFull => write!(f, "Your team is full."),
            GameError::StorageFull => write!(f, "The haven is full."),
            GameError::InvalidSpecies(id) => write!(f, "Unknown creature {}.", id.0),
            GameError::NoMoves => write!(f, "No usable move."),
            GameError::InvalidTransition => write!(f, "Not possible right now."),
        }
    }
}

impl Monster {
    /// A freshly spawned or captured monster with a level-appropriate moveset.
    pub fn wild(id: SpeciesId, level: u8) -> Result<Self, GameError> {
        let data = species(id).ok_or(GameError::InvalidSpecies(id))?;
        let learned: Vec<MoveSlot> = {
            let eligible: Vec<&(&'static str, u8)> = data
                .learnset
                .iter()
                .filter(|(_, lvl)| *lvl <= level)
                .collect();
            let start = eligible.len().saturating_sub(MOVES_PER_MONSTER);
            eligible[start..]
                .iter()
                .map(|(name, _)| MoveSlot {
                    name: (*name).to_string(),
                    current_pp: PP_PER_MOVE,
                    max_pp: PP_PER_MOVE,
                })
                .collect()
        };
        if learned.is_empty() {
            return Err(GameError::NoMoves);
        }
        let stats = stats_at(data, level);
        Ok(Monster {
            species: id,
            nickname: None,
            level,
            experience: 0,
            current_hp: stats.hp,
            moves: learned,
        })
    }

    pub fn data(&self) -> &'static crate::roster::Species {
        species(self.species).expect("saves only carry validated species")
    }

    pub fn name(&self) -> &str {
        self.nickname.as_deref().unwrap_or(self.data().name)
    }

    pub fn max_hp(&self) -> u16 {
        stats_at(self.data(), self.level).hp
    }
}

/// Grown stats at a level. Compact growth: base values scale roughly 2x over
/// the 1..=40 level band, capped to the u16 range.
pub fn stats_at(data: &crate::roster::Species, level: u8) -> Stats {
    let l = level.max(1).clamp(1, MAX_LEVEL) as u32;
    let grow = |base: u16| -> u16 {
        let value = base as u32 * l / 25 + 5;
        value.min(u16::MAX as u32) as u16
    };
    Stats {
        hp: (data.stats.hp as u32 * l / 25 + l + 10).min(u16::MAX as u32) as u16,
        attack: grow(data.stats.attack),
        defense: grow(data.stats.defense),
        speed: data.stats.speed,
    }
}

/// Damage one attacker strike deals, including affinity and level scaling.
/// Deterministic: `roll` drives the small variance band. The caller applies
/// the result to the defender.
pub fn damage(
    attacker: &Monster,
    defender: &Monster,
    move_name: &str,
    roll: u32,
) -> Result<u16, GameError> {
    let slot = attacker
        .moves
        .iter()
        .find(|slot| slot.name == move_name && slot.current_pp > 0)
        .ok_or(GameError::NoMoves)?;
    let data = crate::moves::move_data(&slot.name).ok_or(GameError::NoMoves)?;
    if data.power == 0 {
        // Status moves deal no direct damage in this compact ruleset.
        return Ok(0);
    }
    let move_power = data.power as u32;
    let attack = stats_at(attacker.data(), attacker.level).attack as u32;
    let defense = stats_at(defender.data(), defender.level).defense.max(1) as u32;
    let level_term = 2 * attacker.level as u32 / 5 + 2;
    let base = (level_term * attack * move_power / (50 * defense)).max(1);
    let multiplier = attacker.data().affinity.against(defender.data().affinity);
    let variance = 0.85 + (roll % 16) as f32 / 100.0; // 0.85..=1.00
    let total = (base as f32 * multiplier * variance).round() as u32;
    Ok(total.clamp(1, u16::MAX as u32) as u16)
}
