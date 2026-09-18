//! The authoritative game state: party, storage, items, progression, battles
//! and captures. Serializes as the save attachment; rendering and Android
//! lifecycle never mutate this directly.
use crate::journey::Place;
use crate::monsters::{
    GameError, Monster, MoveSlot, MAX_LEVEL, MAX_PARTY, MAX_STORAGE, MOVES_PER_MONSTER, PP_PER_MOVE,
};
use crate::roster::SpeciesId;
use serde::{Deserialize, Serialize};

pub const STARTER_LEVEL: u8 = 5;
pub const CAPTURE_BASE: u32 = 40; // percent floor at 1 HP, no status
pub const CAPTURE_LEVEL_PENALTY: i32 = 3;
pub const XP_TO_NEXT: u32 = 12;

/// One scene state. The UI drives it through the typed actions below.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Game {
    pub version: u32,
    pub player_name: String,
    pub party: Vec<Monster>,
    pub storage: Vec<Monster>,
    pub capture_charms: u32,
    pub tonics: u32,
    pub defeated_trainers: Vec<String>,
    pub gym_badge: bool,
    pub started: bool,
}

impl Game {
    /// Saves from older or newer format versions are refused, never migrated
    /// silently; recovery keeps the original bytes intact.
    pub fn validate_version(&self) -> Result<(), GameError> {
        if self.version != SAVE_VERSION {
            return Err(GameError::InvalidTransition);
        }
        Ok(())
    }

    /// Deserialize a save, enforcing the version gate.
    pub fn load(bytes: &[u8]) -> Result<Self, GameError> {
        let game: Game = serde_json::from_slice(bytes).map_err(|_| GameError::InvalidTransition)?;
        game.validate()?;
        Ok(game)
    }

    pub fn validate(&self) -> Result<(), GameError> {
        if self.version != SAVE_VERSION {
            return Err(GameError::InvalidTransition);
        }
        if self.party.len() > MAX_PARTY || self.storage.len() > MAX_STORAGE {
            return Err(GameError::InvalidTransition);
        }
        for member in &self.party {
            if member.current_hp > member.max_hp() {
                return Err(GameError::InvalidTransition);
            }
            member.moves.iter().try_for_each(|slot| {
                if slot.current_pp > slot.max_pp {
                    Err(GameError::InvalidTransition)
                } else {
                    Ok(())
                }
            })?;
        }
        Ok(())
    }
}

impl Game {
    pub fn new_game(player_name: &str, starter: SpeciesId) -> Result<Self, GameError> {
        if !crate::roster::STARTERS.contains(&starter) {
            return Err(GameError::InvalidTransition);
        }
        Ok(Game {
            version: SAVE_VERSION,
            player_name: player_name.to_string(),
            party: vec![Monster::wild(starter, STARTER_LEVEL)?],
            storage: Vec::new(),
            capture_charms: 5,
            tonics: 3,
            defeated_trainers: Vec::new(),
            gym_badge: false,
            started: true,
        })
    }

    /// Replenish supplies from a shop or reward. Ordinary play must never be
    /// able to soft-lock on zero charms or tonics.
    pub fn restock(&mut self, charms: u32, tonics: u32) {
        self.capture_charms = self.capture_charms.saturating_add(charms);
        self.tonics = self.tonics.saturating_add(tonics);
    }

    /// Capture a defeated-or-weakened wild monster. Party first, else storage.
    pub fn capture(&mut self, wild: &Monster) -> Result<CapturedWhere, GameError> {
        if self.capture_charms == 0 {
            return Err(GameError::InvalidTransition);
        }
        self.capture_charms -= 1;
        let mut caught = wild.clone();
        caught.current_hp = caught.max_hp() / 2;
        if self.party.len() < MAX_PARTY {
            self.party.push(caught.clone());
            Ok(CapturedWhere::Party)
        } else if self.storage.len() < MAX_STORAGE {
            self.storage.push(caught.clone());
            Ok(CapturedWhere::Storage)
        } else {
            // Refuse the whole capture: the charm was spent, the monster is not.
            self.capture_charms += 1;
            Err(GameError::PartyFull)
        }
    }

    /// Use a tonic on a party member.
    pub fn heal_member(&mut self, index: usize) -> Result<u16, GameError> {
        if self.tonics == 0 {
            return Err(GameError::InvalidTransition);
        }
        let member = self
            .party
            .get_mut(index)
            .ok_or(GameError::InvalidTransition)?;
        let max = member.max_hp();
        if member.current_hp >= max {
            return Err(GameError::InvalidTransition);
        }
        member.current_hp = (member.current_hp + 20).min(max);
        self.tonics -= 1;
        Ok(member.current_hp)
    }

    /// Revive a fainted member at the haven (free, full heal of one member).
    pub fn haven_heal(&mut self) {
        for member in &mut self.party {
            member.current_hp = member.max_hp();
            for slot in &mut member.moves {
                slot.current_pp = slot.max_pp;
            }
        }
    }

    /// Move a monster between party and storage. A fainted monster may not
    /// leave the party with no healthy member behind.
    pub fn swap_storage(&mut self, party_index: usize) -> Result<(), GameError> {
        if party_index >= self.party.len() {
            return Err(GameError::InvalidTransition);
        }
        if self.party.len() == 1 {
            return Err(GameError::InvalidTransition);
        }
        if self.storage.len() >= MAX_STORAGE {
            return Err(GameError::StorageFull);
        }
        let moved = self.party.remove(party_index);
        self.storage.push(moved);
        Ok(())
    }

    pub fn withdraw(&mut self, storage_index: usize) -> Result<(), GameError> {
        if storage_index >= self.storage.len() {
            return Err(GameError::InvalidTransition);
        }
        if self.party.len() >= MAX_PARTY {
            return Err(GameError::PartyFull);
        }
        let monster = self.storage.remove(storage_index);
        self.party.push(monster);
        Ok(())
    }

    /// Grant experience; returns the level reached and any learned move name.
    pub fn grant_xp(
        &mut self,
        party_index: usize,
        amount: u32,
    ) -> Result<(u8, Option<String>), GameError> {
        let member = self
            .party
            .get_mut(party_index)
            .ok_or(GameError::InvalidTransition)?;
        member.experience += amount;
        let mut learned = None;
        let mut leveled = member.level;
        while leveled < MAX_LEVEL && member.experience >= xp_for_next(leveled) {
            member.experience -= xp_for_next(leveled);
            leveled += 1;
            let data = member.data();
            for (name, at) in data.learnset {
                if *at == leveled {
                    let known = member.moves.iter().any(|slot| &slot.name == name);
                    if !known && member.moves.len() < MOVES_PER_MONSTER {
                        member.moves.push(MoveSlot {
                            name: (*name).to_string(),
                            current_pp: PP_PER_MOVE,
                            max_pp: PP_PER_MOVE,
                        });
                        learned = Some((*name).to_string());
                    }
                }
            }
            member.level = leveled;
        }
        member.level = leveled;
        let max = member.max_hp();
        member.current_hp = member.current_hp.min(max);
        Ok((leveled, learned))
    }

    /// Apply evolution when the level gate is met. Returns the new species.
    pub fn evolve_check(&mut self, party_index: usize) -> Result<SpeciesId, GameError> {
        let member = self
            .party
            .get_mut(party_index)
            .ok_or(GameError::InvalidTransition)?;
        let data = member.data();
        let Some((to, required)) = data.evolves_into else {
            return Err(GameError::InvalidTransition);
        };
        if member.level < required {
            return Err(GameError::InvalidTransition);
        }
        let ratio = member.current_hp as f32 / member.max_hp().max(1) as f32;
        member.species = to;
        member.current_hp = (member.max_hp() as f32 * ratio).round() as u16;
        Ok(to)
    }
}

pub const SAVE_VERSION: u32 = 1;

/// The complete on-disk save: rules state plus where the player stands. The
/// maps themselves are deterministic authored data, not saved edits, so a
/// save only needs the position.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveFile {
    pub version: u32,
    pub game: Game,
    pub place: Place,
    pub position: [f32; 3],
    pub yaw: f32,
}

impl SaveFile {
    pub const FILE_VERSION: u32 = 1;

    pub fn new(game: Game, place: Place) -> Self {
        Self {
            version: Self::FILE_VERSION,
            game,
            place,
            position: [0.0; 3],
            yaw: 0.0,
        }
    }

    /// Atomic save: same-directory temp, fsync, rename. A crash cannot leave a
    /// half-written save, and the previous save survives a failed write.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let temp = parent.join(format!(".mossbound-{}.tmp", std::process::id()));
        {
            let mut file = std::fs::File::create(&temp)?;
            let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, path)?;
        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    }

    /// Load with the version gate; a corrupt or foreign file is a refusal, and
    /// the caller keeps the original bytes.
    pub fn load(path: &std::path::Path) -> Result<Self, GameError> {
        let bytes = std::fs::read(path).map_err(|_| GameError::InvalidTransition)?;
        let save: SaveFile =
            serde_json::from_slice(&bytes).map_err(|_| GameError::InvalidTransition)?;
        if save.version != Self::FILE_VERSION {
            return Err(GameError::InvalidTransition);
        }
        save.game.validate()?;
        Ok(save)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturedWhere {
    Party,
    Storage,
}

pub fn xp_for_next(level: u8) -> u32 {
    XP_TO_NEXT * (level as u32 + 1)
}

/// Roll a capture attempt for a wild monster. `roll` in 0..100.
pub fn capture_chance(wild: &Monster) -> u32 {
    let hp_ratio = wild.current_hp as f32 / wild.max_hp().max(1) as f32;
    let hp_bonus = ((1.0 - hp_ratio) * 60.0) as i32;
    let level_penalty = (wild.level as i32 - 5) * CAPTURE_LEVEL_PENALTY;
    (CAPTURE_BASE as i32 + hp_bonus + level_penalty).clamp(5, 95) as u32
}
