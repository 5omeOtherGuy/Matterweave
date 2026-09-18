//! The Mossbound runtime: screens, touch input, overworld walking, encounters,
//! battles and persistence. Game rules live in `matterweave-monsters`; this
//! module owns the window, renderer and input and never mutates world data.
use crate::audio::{GameAudio, Sound};
use crate::lifecycle::{clamped_frame_delta, PlatformEvent, PlatformLifecycle};
use crate::maps::{self, PlaceMap};
use crate::visuals::{self};
use glam::{Mat4, Vec3};

use matterweave_monsters::battle::{
    resolve, Battle, BattleAction, BattleEvent, BattlePhase, Rolls,
};
use matterweave_monsters::journey::{self, Place, Trainer};
use matterweave_monsters::monsters::{Monster, MAX_PARTY};
use matterweave_monsters::roster::SpeciesId;
use matterweave_monsters::state::{Game, SaveFile};
use matterweave_physics::{CharacterProfile, Physics};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, Sun};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, TouchPhase, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

/// Walking speed on the overworld, m/s.
const WALK_SPEED_M_S: f32 = 4.2;
/// Third-person camera distance behind the player, m.
const CAM_DISTANCE_M: f32 = 8.5;
/// Camera pitch looking down at the player, radians.
const CAM_PITCH: f32 = -0.86;
/// Encounters never roll within this many steps of the last battle.
const ENCOUNTER_COOLDOWN_STEPS: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Title,
    StarterPick,
    Overworld,
    Dialogue,
    Battle,
    Menu,
    Ending,
}

/// A registered touch target for the current frame.
#[derive(Clone, Copy, Debug)]
struct Button {
    rect: [f32; 4],
    action: UiAction,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum UiAction {
    NewGame,
    ToggleMute,
    Talk,
    Continue,
    PickStarter(u8),
    Confirm,
    MenuOpen,
    MenuClose,
    BattleMove(u8),
    BattleCapture,
    BattleHeal,
    BattleEscape,
    BattleSwitch,
    MenuHeal(u8),
    MenuSwap(u8),
    MenuWithdraw(u8),
}

struct BattleState {
    battle: Battle,
    /// Remaining trainer monsters after the current one.
    queue: Vec<Monster>,
    trainer: Option<&'static Trainer>,
    active: usize,
    log: Vec<String>,
    flash: f32,
}

/// A scripted host exercise: start a new game, walk a route, meet a wild
/// creature, capture it and save. This drives the real render loop the way a
/// player would; it is host runtime evidence, never a device claim.
pub struct SmokeState {
    pub frames: u64,
    pub battles: u32,
    pub captures: u32,
    pub finished: bool,
}

/// Touch bookkeeping: one joystick drag and one camera drag at a time.
#[derive(Default)]
struct TouchInput {
    joystick: Option<(u64, [f32; 2])>,
    camera: Option<(u64, [f32; 2], f32)>,
    vector: [f32; 2],
    buttons: Vec<Button>,
}

pub struct MonsterApp {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    lifecycle: PlatformLifecycle,
    save_path: PathBuf,
    frame_limit: Option<u64>,
    frames: u64,
    last_frame: Instant,
    pub failed: bool,

    save: Option<SaveFile>,
    map: PlaceMap,
    physics: Physics,
    player_pos: [f32; 3],
    player_yaw: f32,
    cam_yaw: f32,
    screen: Screen,
    rolls: Rolls,
    battle: Option<BattleState>,
    battle_center: [f32; 3],
    dialogue: Vec<String>,
    dirty: bool,
    encounter_cooldown: u32,
    touch: TouchInput,
    message: Option<(String, f32)>,
    uploaded_revision: Option<u64>,
    scene_key: Option<(Place, bool)>,
    pub smoke: Option<SmokeState>,
    audio: GameAudio,
    settings_path: PathBuf,
}

impl MonsterApp {
    pub fn new(save_path: PathBuf, frame_limit: Option<u64>) -> Self {
        let settings_path = save_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|dir| dir.join("mossbound-settings.json"))
            .unwrap_or_else(|| PathBuf::from("mossbound-settings.json"));
        let save = SaveFile::load(&save_path).ok();
        let place = save.as_ref().map(|s| s.place).unwrap_or(Place::Emberfield);
        let map = PlaceMap::build(place, 7);
        let mut physics = Physics::new(&map.world);
        install_profile(&mut physics);
        let (player_pos, player_yaw, cam_yaw, screen, save) = match save {
            Some(save) => {
                let pos = save.position;
                let yaw = save.yaw;
                (pos, yaw, yaw, Screen::Title, Some(save))
            }
            None => {
                let spawn = spawn_world(&map);
                (spawn, 0.0, 0.0, Screen::Title, None)
            }
        };
        physics.teleport([player_pos[0], player_pos[1] + 1.2, player_pos[2]]);
        Self {
            window: None,
            renderer: None,
            lifecycle: PlatformLifecycle::default(),
            save_path,
            frame_limit,
            frames: 0,
            last_frame: Instant::now(),
            failed: false,
            save,
            map,
            physics,
            player_pos,
            player_yaw,
            cam_yaw,
            screen,
            rolls: Rolls::new(0x5EED_1234),
            battle: None,
            battle_center: [16.0, 2.0, 12.0],
            dialogue: Vec::new(),
            dirty: false,
            encounter_cooldown: 0,
            touch: TouchInput::default(),
            message: None,
            uploaded_revision: None,
            scene_key: None,
            smoke: None,
            audio: GameAudio::new(&settings_path),
            settings_path,
        }
    }

    /// Start the scripted host exercise: a fresh game in Emberfield with the
    /// Ember starter, ready to walk the opening route.
    pub fn begin_smoke_exercise(&mut self) {
        let game = Game::new_game("Wren", matterweave_monsters::roster::ids::CINDERUB)
            .expect("the Ember starter is valid");
        self.save = Some(SaveFile::new(game, Place::Emberfield));
        self.install_place(Place::Emberfield, maps::layout(Place::Emberfield).spawn);
        self.cam_yaw = std::f32::consts::PI;
        self.screen = Screen::Overworld;
        self.smoke = Some(SmokeState {
            frames: 0,
            battles: 0,
            captures: 0,
            finished: false,
        });
    }

    /// Drive the scripted exercise for one frame. Returns true when it has
    /// completed and the caller should exit.
    fn drive_smoke(&mut self) -> bool {
        let Some(smoke) = self.smoke.as_mut() else {
            return false;
        };
        smoke.frames += 1;
        let frames = smoke.frames;
        if smoke.finished {
            return true;
        }
        // Count captures from the rules state so the script cannot claim one
        // that did not happen.
        if let Some(save) = &self.save {
            if save.game.party.len() > 1 {
                smoke.captures = smoke.captures.max(1);
            }
        }
        let mut fire_capture = false;
        match self.screen {
            Screen::Overworld => {
                let place = self.map.layout.place;
                let (x, z) = (self.player_pos[0], self.player_pos[2]);
                // Camera yaw is PI: forward is -Z and right is -X.
                self.touch.vector = match place {
                    Place::Emberfield => [0.0, 1.0],
                    _ => {
                        let into_grass = x > 13.0;
                        if into_grass && z > 14.0 {
                            [-1.0, 0.35]
                        } else if z > 10.0 {
                            [0.0, 1.0]
                        } else {
                            [-1.0, 0.0]
                        }
                    }
                };
            }
            Screen::Battle if frames % 20 == 0 => fire_capture = true,
            _ => {}
        }
        if fire_capture {
            smoke.battles += 1;
            self.battle_action(BattleAction::Capture);
        }
        // Done when a capture has landed and the overworld is back with a save.
        let done = self
            .smoke
            .as_ref()
            .is_some_and(|smoke| self.screen == Screen::Overworld && smoke.captures > 0);
        if done {
            self.save_now();
            if let Some(smoke) = self.smoke.as_mut() {
                smoke.finished = true;
                println!(
                    "MOSSBOUND SMOKE: place={} steps={} attempts={} captures={} party={}",
                    self.map.layout.place.name(),
                    smoke.frames,
                    smoke.battles,
                    smoke.captures,
                    self.save.as_ref().map(|s| s.game.party.len()).unwrap_or(0),
                );
            }
            return true;
        }
        false
    }

    fn game(&self) -> Option<&Game> {
        self.save.as_ref().map(|s| &s.game)
    }

    fn game_mut(&mut self) -> Option<&mut Game> {
        self.save.as_mut().map(|s| &mut s.game)
    }

    fn wants_frames(&self) -> bool {
        self.lifecycle.present_allowed()
    }

    fn install_place(&mut self, place: Place, spawn_tile: [i32; 2]) {
        self.map = PlaceMap::build(place, 7);
        self.physics = Physics::new(&self.map.world);
        install_profile(&mut self.physics);
        let spawn = [
            spawn_tile[0] as f32 + 0.5,
            2.0 + 1.2,
            spawn_tile[1] as f32 + 0.5,
        ];
        self.physics.teleport(spawn);
        self.player_pos = [spawn[0], 2.0, spawn[2]];
        self.uploaded_revision = None;
        self.scene_key = None;
        if let Some(save) = &mut self.save {
            save.place = place;
        }
        self.dirty = true;
    }

    fn say(&mut self, lines: Vec<String>) {
        self.dialogue = lines;
        self.screen = Screen::Dialogue;
    }

    fn notice(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), 2.5));
    }

    fn interact(&mut self) {
        let Some(game) = self.game().cloned() else {
            return;
        };
        let tile = [
            self.player_pos[0].floor() as i32,
            self.player_pos[2].floor() as i32,
        ];
        // Interact with an NPC on this tile or one step away.
        let npc = self
            .map
            .npc_at(tile)
            .copied()
            .or_else(|| adjacent_npcs(&self.map, tile).into_iter().next().copied());
        let Some(npc) = npc else {
            return;
        };
        match npc.id {
            "haven" => {
                let game = self.save.as_mut().map(|s| &mut s.game).unwrap();
                game.haven_heal();
                self.dirty = true;
                self.say(vec![
                    "The Haven keeper tends your team.".into(),
                    "Your creatures are fully restored.".into(),
                ]);
            }
            "shop" => {
                let game = self.save.as_mut().map(|s| &mut s.game).unwrap();
                game.restock(5, 2);
                self.dirty = true;
                self.say(vec![
                    "The supply counter restocks you.".into(),
                    "+5 charms, +2 tonics.".into(),
                ]);
            }
            id => {
                // Gym and route trainers: fight in authored order.
                let trainer = trainer_for(id);
                let Some(trainer) = trainer else { return };
                if game.defeated_trainers.iter().any(|t| t == id) {
                    self.say(vec![format!(
                        "{} nods. Their team is resting.",
                        trainer.title
                    )]);
                    return;
                }
                // Gym lead-in order: attendants must fall before the leader.
                if id == journey::GYM_LEADER {
                    let unfinished = journey::GYM_TRAINERS
                        .iter()
                        .any(|g| !game.defeated_trainers.iter().any(|d| d == g));
                    if unfinished {
                        self.say(vec![
                            "Leader Marwick watches. Defeat the attendants first.".into()
                        ]);
                        return;
                    }
                }
                if trainer.post_gym && !game.gym_badge {
                    self.say(vec!["Come back after you earn the Tidewater badge.".into()]);
                    return;
                }
                let mut queue: Vec<Monster> = Vec::new();
                for picked in trainer.party {
                    match Monster::wild(picked.species, picked.level) {
                        Ok(monster) => queue.push(monster),
                        Err(_) => return,
                    }
                }
                let first = queue.remove(0);
                let level = game.party[0].level;
                self.battle = Some(BattleState {
                    battle: Battle::wild_encounter(first, level),
                    queue,
                    trainer: Some(trainer),
                    active: 0,
                    log: Vec::new(),
                    flash: 0.0,
                });
                self.battle_center = [self.player_pos[0], 2.0, self.player_pos[2]];
                self.screen = Screen::Battle;
                self.scene_key = None;
                self.audio.play(Sound::Encounter);
            }
        }
    }

    fn start_wild_battle(&mut self, wild: Monster) {
        self.audio.play(Sound::Encounter);
        let level = self.game().map(|g| g.party[0].level).unwrap_or(5);
        self.battle = Some(BattleState {
            battle: Battle::wild_encounter(wild, level),
            queue: Vec::new(),
            trainer: None,
            active: 0,
            log: Vec::new(),
            flash: 0.0,
        });
        self.battle_center = [self.player_pos[0], 2.0, self.player_pos[2]];
        self.screen = Screen::Battle;
        self.scene_key = None;
        self.touch.vector = [0.0, 0.0];
    }

    /// Advance one overworld step: physics, camera, encounters and exits.
    fn update_overworld(&mut self, dt: f32) {
        if self.game().is_none() {
            return;
        }
        let input = self.touch.vector;
        let forward = [self.cam_yaw.sin(), 0.0, self.cam_yaw.cos()];
        let right = [self.cam_yaw.cos(), 0.0, -self.cam_yaw.sin()];
        let mut velocity = [0.0f32; 3];
        let magnitude = (input[0] * input[0] + input[1] * input[1]).sqrt();
        if magnitude > 0.08 {
            let norm = magnitude.min(1.0);
            for axis in 0..3 {
                velocity[axis] =
                    (forward[axis] * input[1] + right[axis] * input[0]) * norm * WALK_SPEED_M_S;
            }
        }
        let before = self.player_pos;
        if velocity.iter().any(|v| *v != 0.0) {
            self.physics.step(dt, velocity, false);
            let eye = self.physics.character_eye();
            self.player_pos = [eye[0], eye[1] - 1.2, eye[2]];
            self.player_yaw = f32::atan2(velocity[0], velocity[2]);
            self.encounter_cooldown = self.encounter_cooldown.saturating_sub(1);
        } else {
            // Keep gravity honest even while standing still.
            self.physics.step(dt, [0.0; 3], false);
            let eye = self.physics.character_eye();
            self.player_pos = [eye[0], eye[1] - 1.2, eye[2]];
        }

        // Exits: stepping onto an exit tile moves to the next place.
        let tile = [
            self.player_pos[0].floor() as i32,
            self.player_pos[2].floor() as i32,
        ];
        if let Some(exit) = self.map.exit_at(tile) {
            let (target, spawn) = (exit.target, exit.spawn);
            self.install_place(target, spawn);
            self.notice(target.name().to_string());
            return;
        }

        // Wild encounters roll on grass tiles, with a cooldown so a dismissed
        // battle cannot loop immediately.
        let moved = (self.player_pos[0] - before[0]).abs() + (self.player_pos[2] - before[2]).abs();
        if moved > 0.01 && self.encounter_cooldown == 0 {
            if let Some(tile_kind) = self.map.tile(tile[0], tile[1]) {
                let weight = tile_kind.encounter_weight();
                if weight > 0 && self.rolls.percent() < 4 * weight {
                    if let Some(wild) = self.roll_wild() {
                        self.encounter_cooldown = ENCOUNTER_COOLDOWN_STEPS;
                        self.start_wild_battle(wild);
                    }
                }
            }
        }
    }

    fn roll_wild(&mut self) -> Option<Monster> {
        let place = self.map.layout.place;
        let route = journey::route(place)?;
        let badge = self.game().is_some_and(|g| g.gym_badge);
        let table: Vec<_> = route
            .encounters
            .iter()
            .filter(|e| !e.post_gym || badge)
            .collect();
        let total: u32 = table.iter().map(|e| e.weight as u32).sum();
        if total == 0 {
            return None;
        }
        let mut pick = self.rolls.next_u32() % total;
        for entry in &table {
            if pick < entry.weight as u32 {
                let span = entry.max_level.saturating_sub(entry.min_level) as u32 + 1;
                let level = entry.min_level + (self.rolls.next_u32() % span) as u8;
                return Monster::wild(entry.species, level).ok();
            }
            pick -= entry.weight as u32;
        }
        None
    }

    /// Resolve one player battle action and fold the outcome back into the
    /// overworld state.
    fn battle_action(&mut self, action: BattleAction) {
        let MonsterApp {
            battle: battle_slot,
            save,
            rolls,
            ..
        } = self;
        let Some(state) = battle_slot.as_mut() else {
            return;
        };
        let (active, trainer) = (state.active, state.trainer);
        let Some(game) = save.as_mut().map(|s| &mut s.game) else {
            return;
        };
        if trainer.is_some() && matches!(action, BattleAction::Capture | BattleAction::Escape) {
            state.log.push("You cannot flee a trainer battle.".into());
            return;
        }
        let battle = &mut state.battle;
        let before = battle.phase;
        if let Err(error) = resolve(game, battle, active, action, rolls) {
            state.log.push(error.to_string());
            state.log.truncate(6);
            return;
        }
        let mut sound: Option<Sound> = None;
        for event in battle.log.drain(..) {
            match &event {
                BattleEvent::PlayerUsedMove { .. } | BattleEvent::WildUsedMove { .. } => {
                    sound = Some(Sound::Hit)
                }
                BattleEvent::LevelUp { .. } | BattleEvent::Evolved { .. } => {
                    sound = Some(Sound::LevelUp)
                }
                BattleEvent::Heal { .. } => sound = Some(Sound::Heal),
                _ => {}
            }
            state.log.push(describe(&event, game, active));
        }
        if let Some(sound) = sound {
            self.audio.play(sound);
        }
        state.log.truncate(6);
        state.flash = if state.log.last().is_some_and(|line| line.contains("hit")) {
            0.25
        } else {
            0.0
        };
        // Level-up / evolution events are already applied by the battle layer;
        // keep them visible in the log.
        if battle.phase != BattlePhase::PlayerChoice {
            self.finish_or_continue_battle(before);
        }
        self.dirty = true;
    }

    fn finish_or_continue_battle(&mut self, _before: BattlePhase) {
        let MonsterApp {
            battle: battle_slot,
            save,
            ..
        } = self;
        let Some(state) = battle_slot.as_mut() else {
            return;
        };
        let party_level = save.as_ref().map(|s| s.game.party[0].level).unwrap_or(5);
        let phase = state.battle.phase;
        match phase {
            BattlePhase::Won => {
                if let Some(next) = state.queue.first().cloned() {
                    state.queue.remove(0);
                    let level = party_level;
                    state.battle = Battle::wild_encounter(next, level);
                    state.log.push("The next challenger steps forward!".into());
                    state.log.truncate(6);
                    self.scene_key = None;
                } else if let Some(trainer) = state.trainer {
                    let (charms, tonics) = (trainer.reward_charms, trainer.reward_tonics);
                    let title = trainer.title;
                    let id = trainer.id.to_string();
                    let leader = trainer.id == journey::GYM_LEADER;
                    if let Some(game) = save.as_mut().map(|s| &mut s.game) {
                        if !game.defeated_trainers.contains(&id) {
                            game.defeated_trainers.push(id.clone());
                            game.restock(charms, tonics);
                        }
                        if leader {
                            game.gym_badge = true;
                        }
                    }
                    let mut lines = vec![format!("{title} is defeated!")];
                    if leader {
                        lines.push("You earned the Tidewater badge!".into());
                        lines.push("The Quarry Loop sanctuary is open.".into());
                    } else if charms > 0 {
                        lines.push(format!("Reward: +{charms} charms, +{tonics} tonics."));
                    }
                    if leader {
                        self.audio.play(Sound::Badge);
                    }
                    self.battle = None;
                    self.screen = Screen::Overworld;
                    self.scene_key = None;
                    self.dirty = true;
                    self.say(lines);
                    if leader {
                        self.screen = Screen::Ending;
                    }
                } else {
                    self.battle = None;
                    self.screen = Screen::Overworld;
                    self.scene_key = None;
                    self.dirty = true;
                    let caught = self.save.as_ref().map(|s| s.game.party.len()).unwrap_or(0);
                    if caught > 0 {
                        self.notice(format!("The wild creature was caught! Team: {caught}."));
                    } else {
                        self.notice("The wild creature was defeated!");
                    }
                }
            }
            BattlePhase::Captured => {
                self.audio.play(Sound::Capture);
                self.battle = None;
                self.screen = Screen::Overworld;
                self.scene_key = None;
                self.dirty = true;
                self.notice("Captured! It joined your team.");
            }
            BattlePhase::Fled => {
                self.battle = None;
                self.screen = Screen::Overworld;
                self.scene_key = None;
                self.dirty = true;
                self.say(vec!["You slipped away safely.".into()]);
            }
            BattlePhase::Lost => {
                self.battle = None;
                self.scene_key = None;
                self.screen = Screen::Overworld;
                if let Some(game) = self.game_mut() {
                    game.haven_heal();
                }
                self.install_place(Place::Emberfield, maps::layout(Place::Emberfield).spawn);
                self.say(vec![
                    "Your team fainted...".into(),
                    "The Haven keeper carries you back to Emberfield.".into(),
                ]);
            }
            BattlePhase::PlayerChoice => {}
        }
    }

    /// Index of the next conscious party member after the active one.
    fn next_healthy_member(&self) -> Option<usize> {
        let state = self.battle.as_ref()?;
        let game = self.game()?;
        let count = game.party.len();
        for offset in 1..=count {
            let index = (state.active + offset) % count;
            if index != state.active && game.party[index].current_hp > 0 {
                return Some(index);
            }
        }
        None
    }

    fn menu_action(&mut self, action: UiAction) {
        match action {
            UiAction::MenuHeal(index) => {
                let index = index as usize;
                let outcome = self
                    .save
                    .as_mut()
                    .map(|s| s.game.heal_member(index))
                    .transpose();
                match outcome {
                    Ok(Some(restored)) => {
                        self.dirty = true;
                        let name = self
                            .save
                            .as_ref()
                            .and_then(|s| s.game.party.get(index))
                            .map(|m| m.name().to_string())
                            .unwrap_or_default();
                        self.notice(format!("{name} restored to {restored} HP."));
                    }
                    Ok(None) => {}
                    Err(error) => self.notice(error.to_string()),
                }
            }
            UiAction::MenuSwap(index) => {
                let result = self
                    .save
                    .as_mut()
                    .map(|s| s.game.swap_storage(index as usize));
                match result {
                    Some(Ok(())) => {
                        self.dirty = true;
                        self.notice("Moved to the haven.");
                    }
                    Some(Err(error)) => self.notice(error.to_string()),
                    None => {}
                }
            }
            UiAction::MenuWithdraw(index) => {
                let result = self.save.as_mut().map(|s| s.game.withdraw(index as usize));
                match result {
                    Some(Ok(())) => {
                        self.dirty = true;
                        self.notice("Joined your team.");
                    }
                    Some(Err(error)) => self.notice(error.to_string()),
                    None => {}
                }
            }
            _ => {}
        }
    }

    fn button_action(&mut self, action: UiAction) {
        match action {
            UiAction::NewGame => {
                self.screen = Screen::StarterPick;
            }
            UiAction::Continue => {
                if let Some(save) = self.save.clone() {
                    self.install_place(save.place, tile_of(save.position));
                    self.player_pos = save.position;
                    self.player_yaw = save.yaw;
                    self.cam_yaw = save.yaw;
                    self.screen = Screen::Overworld;
                    self.notice("Your journey continues.");
                } else {
                    self.screen = Screen::StarterPick;
                }
            }
            UiAction::PickStarter(index) => {
                let starters = matterweave_monsters::roster::STARTERS;
                let Some(id) = starters.get(index as usize).copied() else {
                    return;
                };
                match Game::new_game("Wren", id) {
                    Ok(game) => {
                        self.save = Some(SaveFile::new(game, Place::Emberfield));
                        self.install_place(
                            Place::Emberfield,
                            maps::layout(Place::Emberfield).spawn,
                        );
                        self.screen = Screen::Overworld;
                        self.dirty = true;
                        let name = matterweave_monsters::roster::species(id)
                            .map(|s| s.name)
                            .unwrap_or("?");
                        self.say(vec![
                            format!("{name} joins you!"),
                            "Press ACTION near people to talk.".into(),
                            "Walk in tall grass to meet wild creatures.".into(),
                        ]);
                    }
                    Err(error) => self.notice(error.to_string()),
                }
            }
            UiAction::Talk => {
                if self.screen == Screen::Overworld {
                    self.interact();
                }
            }
            UiAction::Confirm => {
                if self.screen == Screen::Dialogue || self.screen == Screen::Ending {
                    self.screen = Screen::Overworld;
                    self.dialogue.clear();
                }
            }
            UiAction::MenuOpen => {
                if self.screen == Screen::Overworld {
                    self.screen = Screen::Menu;
                }
            }
            UiAction::MenuClose => {
                if self.screen == Screen::Menu {
                    self.screen = Screen::Overworld;
                    self.dirty = true;
                }
            }
            UiAction::BattleMove(index) => {
                self.battle_action(BattleAction::Fight(index as usize));
            }
            UiAction::BattleCapture => self.battle_action(BattleAction::Capture),
            UiAction::BattleHeal => {
                let active = self.battle.as_ref().map(|b| b.active).unwrap_or(0);
                self.battle_action(BattleAction::Heal(active));
            }
            UiAction::BattleEscape => self.battle_action(BattleAction::Escape),
            UiAction::BattleSwitch => {
                let next = self.next_healthy_member();
                if let Some(index) = next {
                    self.battle_action(BattleAction::Switch(index));
                } else {
                    self.notice("No other creature can fight.");
                }
            }
            UiAction::ToggleMute => {
                self.audio.set_muted(!self.audio.settings.muted);
                if let Err(error) = self.audio.settings.save(&self.settings_path) {
                    log::warn!("settings save failed: {error}");
                }
                let muted = self.audio.settings.muted;
                self.notice(if muted { "Sound off." } else { "Sound on." });
            }
            UiAction::MenuHeal(_) | UiAction::MenuSwap(_) | UiAction::MenuWithdraw(_) => {
                self.menu_action(action)
            }
        }
        if matches!(self.screen, Screen::Ending) && action == UiAction::Confirm {
            self.screen = Screen::Overworld;
        }
    }

    fn save_now(&mut self) {
        let Some(save) = self.save.as_mut() else {
            return;
        };
        save.position = self.player_pos;
        save.yaw = self.player_yaw;
        save.place = self.map.layout.place;
        if let Err(error) = save.save(&self.save_path) {
            log::warn!("save failed: {error}");
        } else {
            self.dirty = false;
        }
    }

    fn sync_render_meshes(&mut self) -> Result<(), String> {
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        let revision = self.map.world.revision();
        if self.uploaded_revision != Some(revision) {
            let keys = self.map.world.chunk_keys();
            renderer.retain_chunks(&keys).map_err(|e| e.to_string())?;
            for key in keys {
                renderer
                    .upload_chunk(key, &self.map.world.mesh_chunk(key))
                    .map_err(|e| e.to_string())?;
            }
            self.uploaded_revision = Some(revision);
        }
        Ok(())
    }

    fn sync_scene(&mut self) -> Result<(), String> {
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        let battle = self.screen == Screen::Battle;
        let key = (self.map.layout.place, battle);
        if self.scene_key == Some(key) {
            return Ok(());
        }
        let (meshes, instances) = if battle {
            let Some(state) = self.battle.as_ref() else {
                return Ok(());
            };
            let player_species = self
                .save
                .as_ref()
                .and_then(|s| s.game.party.get(state.active))
                .map(|m| m.species)
                .unwrap_or(SpeciesId(1));
            let wild_species = state.battle.wild.species;
            visuals::arena_scene(player_species, wild_species, self.battle_center, 0.045)
        } else {
            let mut npcs = Vec::new();
            for spot in self.map.layout.npcs {
                let role = visuals::role_of(spot.id);
                let y = 2.0;
                npcs.push((
                    role,
                    [spot.tile[0] as f32 + 0.5, y, spot.tile[1] as f32 + 0.5],
                    0u8,
                ));
            }
            visuals::npc_scene(&npcs)
        };
        renderer
            .replace_static_scene(&meshes, &instances)
            .map_err(|e| e.to_string())?;
        self.scene_key = Some(key);
        Ok(())
    }

    fn camera(&self) -> (Mat4, [f32; 3]) {
        if self.screen == Screen::Battle {
            let center = Vec3::from(self.battle_center);
            let eye = center + Vec3::new(0.0, 5.0, -7.6);
            let target = center + Vec3::new(0.0, 1.1, 0.0);
            (Mat4::look_at_rh(eye, target, Vec3::Y), eye.to_array())
        } else {
            let target = Vec3::new(
                self.player_pos[0],
                self.player_pos[1] + 1.1,
                self.player_pos[2],
            );
            let forward = Vec3::new(
                self.cam_yaw.sin() * CAM_PITCH.cos(),
                CAM_PITCH.sin(),
                self.cam_yaw.cos() * CAM_PITCH.cos(),
            );
            let eye = target - forward * CAM_DISTANCE_M;
            (Mat4::look_at_rh(eye, target, Vec3::Y), eye.to_array())
        }
    }

    fn view_proj(&self, aspect: f32) -> [[f32; 4]; 4] {
        let projection = Mat4::perspective_rh(50f32.to_radians(), aspect.max(0.01), 0.1, 200.0);
        (projection * self.camera().0).to_cols_array_2d()
    }

    fn player_avatar_mesh(&self) -> matterweave_core::Mesh {
        let mut mesh = visuals::player_mesh();
        let (sin, cos) = self.player_yaw.sin_cos();
        for vertex in &mut mesh.vertices {
            let [x, y, z] = vertex.position;
            vertex.position = [
                x * cos + z * sin + self.player_pos[0],
                y + self.player_pos[1],
                -x * sin + z * cos + self.player_pos[2],
            ];
            let [nx, ny, nz] = vertex.normal;
            vertex.normal = [nx * cos + nz * sin, ny, -nx * sin + nz * cos];
        }
        mesh
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if !self.wants_frames() {
            return;
        }
        let now = Instant::now();
        let dt = clamped_frame_delta(self.last_frame, now).clamp(0.001, 0.05);
        self.last_frame = now;
        if let Some((_, until)) = &mut self.message {
            *until -= dt;
            if *until <= 0.0 {
                self.message = None;
            }
        }
        if let Some(state) = &mut self.battle {
            state.flash = (state.flash - dt).max(0.0);
        }

        self.audio.poll();
        if self.smoke.is_some() && self.drive_smoke() {
            event_loop.exit();
            return;
        }
        if self.screen == Screen::Overworld {
            self.update_overworld(dt);
        }
        if self.dirty && self.screen == Screen::Overworld {
            self.save_now();
        }

        if let Err(error) = self.sync_render_meshes() {
            log::error!("mesh sync failed: {error}");
            self.failed = true;
            event_loop.exit();
            return;
        }
        if let Err(error) = self.sync_scene() {
            log::error!("scene build failed: {error}");
            self.failed = true;
            event_loop.exit();
            return;
        }

        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        let width = size.width.max(1) as f32;
        let height = size.height.max(1) as f32;
        let aspect = width / height;

        let mut hud = Hud::new(width, height);
        self.touch.buttons.clear();
        self.draw_hud(&mut hud, width, height);

        let (view_proj, eye) = (self.view_proj(aspect), self.camera().1);
        let lighting = LightingSettings {
            sun: Sun {
                direction_to_sun: [0.42, 0.78, 0.33],
                intensity: 0.95,
            },
            shadows: true,
            shadow_map_size: 1024,
            ..Default::default()
        };

        let avatar = if self.screen != Screen::Battle {
            self.player_avatar_mesh()
        } else {
            matterweave_core::Mesh {
                vertices: Vec::new(),
                indices: Vec::new(),
                revision: 0,
            }
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if let Err(error) = renderer.upload_dynamic(&avatar) {
            log::warn!("avatar upload failed: {error}");
        }

        match renderer.render_with_lighting(view_proj, eye, &hud, &lighting) {
            FrameResult::Presented | FrameResult::Retry => {}
            FrameResult::OutOfMemory => log::warn!("out of memory during frame"),
            FrameResult::Fatal(error) => {
                log::error!("Vulkan fatal: {error}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        }
        self.frames += 1;
        if let Some(limit) = self.frame_limit {
            if self.frames >= limit {
                event_loop.exit();
            }
        }
    }

    fn button(&mut self, rect: [f32; 4], label: &str, action: UiAction, hud: &mut Hud) {
        hud.rect(rect, [0.09, 0.10, 0.14, 0.86]);
        hud.rect(
            [rect[0] + 2.0, rect[1] + 2.0, rect[2] - 4.0, 2.0],
            [0.55, 0.75, 0.95, 0.9],
        );
        let scale = (rect[3] / 12.0).clamp(1.5, 3.0);
        let text_w = label.len() as f32 * 8.0 * scale;
        hud.text(
            rect[0] + (rect[2] - text_w).max(6.0) / 2.0,
            rect[1] + rect[3] / 2.0 - 4.0 * scale,
            label,
            scale,
            [0.95, 0.96, 0.98, 1.0],
        );
        self.touch.buttons.push(Button { rect, action });
    }

    fn draw_hud(&mut self, hud: &mut Hud, width: f32, height: f32) {
        let pad = 12.0;
        match self.screen {
            Screen::Title => {
                hud.rect([0.0, 0.0, width, height], [0.03, 0.05, 0.09, 0.55]);
                hud.text(
                    width / 2.0 - 170.0,
                    height * 0.16,
                    "MOSSBOUND",
                    6.0,
                    [0.96, 0.9, 0.6, 1.0],
                );
                hud.text(
                    width / 2.0 - 150.0,
                    height * 0.16 + 60.0,
                    "a monster-catching journey",
                    2.0,
                    [0.85, 0.88, 0.92, 1.0],
                );
                let bw = 300.0;
                let bx = width / 2.0 - bw / 2.0;
                self.button(
                    [bx, height * 0.45, bw, 64.0],
                    "NEW GAME",
                    UiAction::NewGame,
                    hud,
                );
                if self.save.is_some() {
                    self.button(
                        [bx, height * 0.45 + 84.0, bw, 64.0],
                        "CONTINUE",
                        UiAction::Continue,
                        hud,
                    );
                }
            }
            Screen::StarterPick => {
                hud.rect([0.0, 0.0, width, height], [0.03, 0.05, 0.09, 0.6]);
                hud.text(
                    pad,
                    pad,
                    "CHOOSE YOUR FIRST CREATURE",
                    3.0,
                    [0.95, 0.96, 0.98, 1.0],
                );
                let starters = matterweave_monsters::roster::STARTERS;
                for (index, id) in starters.iter().enumerate() {
                    let data = matterweave_monsters::roster::species(*id).unwrap();
                    let bw = 300.0;
                    let bh = 90.0;
                    let bx = width / 2.0 - bw / 2.0;
                    let by = height * 0.28 + index as f32 * (bh + 18.0);
                    self.button(
                        [bx, by, bw, bh],
                        data.name,
                        UiAction::PickStarter(index as u8),
                        hud,
                    );
                    self.button(
                        [bx, by + bh + 2.0, bw, 22.0],
                        affinity_label(data.affinity),
                        UiAction::PickStarter(index as u8),
                        hud,
                    );
                }
            }
            Screen::Overworld | Screen::Dialogue | Screen::Menu | Screen::Ending => {
                self.draw_overworld_hud(hud, width, height);
            }
            Screen::Battle => self.draw_battle_hud(hud, width, height),
        }
    }

    fn draw_overworld_hud(&mut self, hud: &mut Hud, width: f32, height: f32) {
        let Some(save) = self.save.clone() else {
            return;
        };
        let pad = 12.0;
        hud.rect([pad, pad, 300.0, 84.0], [0.05, 0.07, 0.11, 0.8]);
        hud.text(
            pad + 8.0,
            pad + 8.0,
            self.map.layout.place.name(),
            2.6,
            [0.95, 0.96, 0.98, 1.0],
        );
        hud.text(
            pad + 8.0,
            pad + 34.0,
            &format!(
                "charms {}  tonics {}",
                save.game.capture_charms, save.game.tonics
            ),
            2.0,
            [0.85, 0.88, 0.92, 1.0],
        );
        if save.game.gym_badge {
            hud.text(
                pad + 8.0,
                pad + 58.0,
                "BADGE: TIDEWATER",
                2.0,
                [0.98, 0.85, 0.4, 1.0],
            );
        }

        // Party strip along the top right.
        let mut x = width - pad - 200.0;
        for (index, member) in save.game.party.iter().enumerate().take(MAX_PARTY) {
            let y = pad;
            hud.rect([x, y, 190.0, 44.0], [0.05, 0.07, 0.11, 0.82]);
            hud.text(
                x + 6.0,
                y + 4.0,
                member.name(),
                2.0,
                [0.95, 0.96, 0.98, 1.0],
            );
            let ratio = member.current_hp as f32 / member.max_hp().max(1) as f32;
            let color = if ratio > 0.5 {
                [0.35, 0.85, 0.45, 1.0]
            } else if ratio > 0.2 {
                [0.95, 0.8, 0.35, 1.0]
            } else {
                [0.9, 0.35, 0.35, 1.0]
            };
            hud.rect([x + 6.0, y + 24.0, 178.0, 10.0], [0.12, 0.14, 0.18, 1.0]);
            hud.rect([x + 6.0, y + 24.0, 178.0 * ratio, 10.0], color);
            hud.text(
                x + 6.0,
                y + 34.0,
                &format!(
                    "L{} {}/{}",
                    member.level,
                    member.current_hp,
                    member.max_hp()
                ),
                1.4,
                [0.8, 0.84, 0.9, 1.0],
            );
            x -= 200.0;
            if index + 1 >= save.game.party.len() {
                break;
            }
        }

        // Menu button.
        self.button(
            [width - pad - 110.0, height - pad - 54.0, 110.0, 54.0],
            "MENU",
            UiAction::MenuOpen,
            hud,
        );
        // Action button: talk, confirm or dismiss.
        let action_label = match self.screen {
            Screen::Dialogue => "OK",
            Screen::Ending => "CONTINUE",
            _ => "TALK",
        };
        self.button(
            [width - pad - 110.0, height - pad - 130.0, 110.0, 64.0],
            action_label,
            if self.screen == Screen::Dialogue || self.screen == Screen::Ending {
                UiAction::Confirm
            } else {
                UiAction::Talk
            },
            hud,
        );
        if self.screen == Screen::Overworld {
            // TALK triggers interaction directly on press; draw a hint instead.
            hud.text(
                width - pad - 210.0,
                height - pad - 126.0,
                "walk into grass to meet creatures",
                1.4,
                [0.85, 0.88, 0.92, 0.9],
            );
        }

        if let Some((text, _)) = &self.message {
            hud.rect(
                [width / 2.0 - 260.0, height * 0.12, 520.0, 46.0],
                [0.05, 0.07, 0.11, 0.85],
            );
            hud.text(
                width / 2.0 - 250.0,
                height * 0.12 + 12.0,
                text,
                2.2,
                [0.95, 0.97, 0.99, 1.0],
            );
        }

        if self.screen == Screen::Dialogue {
            let box_y = height - pad - 220.0;
            hud.rect(
                [pad, box_y, width - pad * 2.0 - 130.0, 120.0],
                [0.05, 0.07, 0.11, 0.9],
            );
            for (index, line) in self.dialogue.iter().enumerate() {
                hud.text(
                    pad + 14.0,
                    box_y + 14.0 + index as f32 * 30.0,
                    line,
                    2.4,
                    [0.95, 0.97, 0.99, 1.0],
                );
            }
        }

        if self.screen == Screen::Ending {
            hud.rect(
                [0.0, height * 0.3, width, height * 0.4],
                [0.03, 0.05, 0.09, 0.88],
            );
            hud.text(
                width / 2.0 - 230.0,
                height * 0.36,
                "TIDEWATER BADGE EARNED",
                4.0,
                [0.98, 0.86, 0.4, 1.0],
            );
            hud.text(
                width / 2.0 - 250.0,
                height * 0.36 + 54.0,
                "The sanctuary on the Quarry Loop now hosts rare species.",
                2.0,
                [0.9, 0.93, 0.96, 1.0],
            );
            hud.text(
                width / 2.0 - 250.0,
                height * 0.36 + 84.0,
                "Complete the roster, raise final forms, explore the loop.",
                2.0,
                [0.9, 0.93, 0.96, 1.0],
            );
        }

        if self.screen == Screen::Menu {
            self.draw_menu(hud, width, height, &save);
        }
    }

    fn draw_menu(&mut self, hud: &mut Hud, width: f32, height: f32, save: &SaveFile) {
        let pad = 12.0;
        hud.rect([0.0, 0.0, width, height], [0.03, 0.04, 0.06, 0.7]);
        hud.text(pad, pad, "TEAM & HAVEN", 3.0, [0.95, 0.96, 0.98, 1.0]);
        for (index, member) in save.game.party.iter().enumerate() {
            let y = pad + 60.0 + index as f32 * 78.0;
            hud.rect([pad, y, width * 0.52, 70.0], [0.06, 0.08, 0.12, 0.9]);
            hud.text(
                pad + 10.0,
                y + 8.0,
                &format!("{}  L{}", member.name(), member.level),
                2.4,
                [0.95, 0.96, 0.98, 1.0],
            );
            hud.text(
                pad + 10.0,
                y + 34.0,
                &format!("HP {}/{}", member.current_hp, member.max_hp()),
                2.0,
                [0.85, 0.88, 0.92, 1.0],
            );
            self.button(
                [width * 0.55, y + 6.0, 150.0, 56.0],
                "TONIC",
                UiAction::MenuHeal(index as u8),
                hud,
            );
            self.button(
                [width * 0.55 + 160.0, y + 6.0, 150.0, 56.0],
                "STORE",
                UiAction::MenuSwap(index as u8),
                hud,
            );
        }
        let store_y = pad + 60.0 + save.game.party.len() as f32 * 78.0;
        hud.text(
            pad,
            store_y,
            &format!(
                "HAVEN ({}/{})",
                save.game.storage.len(),
                matterweave_monsters::monsters::MAX_STORAGE
            ),
            2.4,
            [0.9, 0.92, 0.95, 1.0],
        );
        for (index, member) in save.game.storage.iter().enumerate().take(8) {
            let y = store_y + 30.0 + index as f32 * 52.0;
            hud.rect([pad, y, width * 0.52, 46.0], [0.06, 0.08, 0.12, 0.85]);
            hud.text(
                pad + 10.0,
                y + 10.0,
                &format!("{} L{}", member.name(), member.level),
                2.2,
                [0.9, 0.92, 0.95, 1.0],
            );
            self.button(
                [width * 0.55, y + 4.0, 150.0, 40.0],
                "WITHDRAW",
                UiAction::MenuWithdraw(index as u8),
                hud,
            );
        }
        let sound_label = if self.audio.settings.muted {
            "SOUND OFF"
        } else {
            "SOUND ON"
        };
        self.button(
            [width - 130.0, height - 140.0, 118.0, 58.0],
            sound_label,
            UiAction::ToggleMute,
            hud,
        );
        self.button(
            [width - 130.0, height - 70.0, 118.0, 58.0],
            "CLOSE",
            UiAction::MenuClose,
            hud,
        );
    }

    fn draw_battle_hud(&mut self, hud: &mut Hud, width: f32, height: f32) {
        // Copy the small presentation set out of the state first so the HUD
        // can register buttons mutably without holding a game borrow.
        let (wild, log, active, moves) = {
            let Some(state) = self.battle.as_ref() else {
                return;
            };
            let Some(game) = self.game() else { return };
            let active = state.active.min(game.party.len().saturating_sub(1));
            let member = game.party.get(active);
            let moves: Vec<(String, u8, u8)> = member
                .map(|m| {
                    m.moves
                        .iter()
                        .take(4)
                        .map(|slot| (slot.name.clone(), slot.current_pp, slot.max_pp))
                        .collect()
                })
                .unwrap_or_default();
            (
                Some((
                    state.battle.wild.name().to_string(),
                    state.battle.wild.level,
                    state.battle.wild.current_hp,
                    state.battle.wild.max_hp(),
                )),
                state.log.clone(),
                active,
                moves,
            )
        };
        let member_info = {
            let Some(game) = self.game() else { return };
            game.party
                .get(active)
                .map(|m| (m.name().to_string(), m.level, m.current_hp, m.max_hp()))
        };
        let pad = 12.0;
        let member = member_info.as_ref();

        // Player card.
        hud.rect([pad, height - 210.0, 330.0, 92.0], [0.05, 0.07, 0.11, 0.9]);
        if let Some((name, level, hp, max_hp)) = member {
            hud.text(
                pad + 10.0,
                height - 200.0,
                name,
                2.6,
                [0.95, 0.96, 0.98, 1.0],
            );
            let ratio = *hp as f32 / (*max_hp).max(1) as f32;
            hud.rect(
                [pad + 10.0, height - 168.0, 300.0, 12.0],
                [0.12, 0.14, 0.18, 1.0],
            );
            hud.rect(
                [pad + 10.0, height - 168.0, 300.0 * ratio, 12.0],
                [0.35, 0.85, 0.45, 1.0],
            );
            hud.text(
                pad + 10.0,
                height - 148.0,
                &format!("L{level}  HP {hp}/{max_hp}"),
                2.0,
                [0.85, 0.88, 0.92, 1.0],
            );
        }

        // Opponent card.
        if let Some((name, level, hp, max_hp)) = &wild {
            hud.rect([width - 340.0, pad, 330.0, 92.0], [0.05, 0.07, 0.11, 0.9]);
            hud.text(
                width - 330.0,
                pad + 10.0,
                name,
                2.6,
                [0.95, 0.96, 0.98, 1.0],
            );
            let wild_ratio = *hp as f32 / (*max_hp).max(1) as f32;
            hud.rect(
                [width - 330.0, pad + 42.0, 300.0, 12.0],
                [0.12, 0.14, 0.18, 1.0],
            );
            hud.rect(
                [width - 330.0, pad + 42.0, 300.0 * wild_ratio, 12.0],
                [0.9, 0.4, 0.4, 1.0],
            );
            hud.text(
                width - 330.0,
                pad + 60.0,
                &format!("L{level}  HP {hp}/{max_hp}"),
                2.0,
                [0.85, 0.88, 0.92, 1.0],
            );
        }

        // Log.
        let log_y = height - 210.0;
        hud.rect(
            [pad + 350.0, log_y, width - 700.0 - pad * 2.0, 120.0],
            [0.04, 0.05, 0.08, 0.85],
        );
        for (index, line) in log.iter().rev().take(3).enumerate() {
            hud.text(
                pad + 360.0,
                log_y + 12.0 + index as f32 * 32.0,
                line,
                2.0,
                [0.92, 0.94, 0.97, 1.0],
            );
        }

        // Action buttons.
        let bw = 200.0;
        let bh = 54.0;
        let bx = width - pad - bw;
        let mut by = height - 210.0;
        for (index, (name, pp, _max)) in moves.iter().enumerate() {
            let label = format!("{name} {pp}");
            self.button(
                [bx - (index % 2) as f32 * (bw + 8.0), by, bw, bh],
                &label,
                UiAction::BattleMove(index as u8),
                hud,
            );
            if index % 2 == 1 {
                by -= bh + 8.0;
            }
        }
        by = pad + 110.0;
        self.button([bx, by, bw, bh], "CAPTURE", UiAction::BattleCapture, hud);
        self.button(
            [bx - bw - 8.0, by, bw, bh],
            "TONIC",
            UiAction::BattleHeal,
            hud,
        );
        self.button(
            [bx - bw - 8.0, by + bh + 8.0, bw, bh],
            "RUN",
            UiAction::BattleEscape,
            hud,
        );
        self.button(
            [bx, by + bh + 8.0, bw, bh],
            "SWITCH",
            UiAction::BattleSwitch,
            hud,
        );
    }

    /// Handle a touch press at a point: registered HUD buttons first, then
    /// the movement or camera half of the screen.
    fn pointer_down(&mut self, id: u64, point: [f32; 2], width: f32) {
        if let Some(button) = self
            .touch
            .buttons
            .iter()
            .rev()
            .copied()
            .find(|b| contains(b.rect, point))
        {
            self.button_action(button.action);
            return;
        }
        if self.screen == Screen::Battle || self.screen == Screen::Menu {
            return;
        }
        if point[0] < width * 0.45 {
            self.touch.joystick = Some((id, point));
        } else {
            self.touch.camera = Some((id, point, self.cam_yaw));
        }
    }

    fn pointer_move(&mut self, id: u64, point: [f32; 2]) {
        if let Some((active, origin)) = self.touch.joystick {
            if active == id {
                let dx = (point[0] - origin[0]) / 90.0;
                let dy = (origin[1] - point[1]) / 90.0;
                let magnitude = (dx * dx + dy * dy).sqrt().max(1.0);
                self.touch.vector = [dx / magnitude, dy / magnitude];
            }
        }
        if let Some((active, origin, base_yaw)) = self.touch.camera {
            if active == id {
                self.cam_yaw = base_yaw - (point[0] - origin[0]) * 0.006;
            }
        }
    }

    fn pointer_up(&mut self, id: u64) {
        if self.touch.joystick.is_some_and(|(active, _)| active == id) {
            self.touch.joystick = None;
            self.touch.vector = [0.0, 0.0];
        }
        if self.touch.camera.is_some_and(|(active, _, _)| active == id) {
            self.touch.camera = None;
        }
    }
}

fn install_profile(physics: &mut Physics) {
    let profile = CharacterProfile {
        eye_height_m: 1.2,
        jump_height_m: 0.0,
        autostep_height_m: 1.05,
        autostep_min_width_m: 0.2,
        bounds_m: 30.0,
    };
    assert!(profile.valid(), "the walking profile is in range");
    physics.set_character_profile(profile);
}

fn spawn_world(map: &PlaceMap) -> [f32; 3] {
    let spawn = map.layout.spawn;
    [spawn[0] as f32 + 0.5, 2.0, spawn[1] as f32 + 0.5]
}

fn tile_of(position: [f32; 3]) -> [i32; 2] {
    [position[0].floor() as i32, position[2].floor() as i32]
}

fn contains(rect: [f32; 4], point: [f32; 2]) -> bool {
    point[0] >= rect[0]
        && point[0] <= rect[0] + rect[2]
        && point[1] >= rect[1]
        && point[1] <= rect[1] + rect[3]
}

fn adjacent_npcs(map: &PlaceMap, tile: [i32; 2]) -> Vec<&'static maps::NpcSpot> {
    map.layout
        .npcs
        .iter()
        .filter(|npc| {
            let dx = (npc.tile[0] - tile[0]).abs();
            let dz = (npc.tile[1] - tile[1]).abs();
            dx <= 1 && dz <= 1 && (dx + dz) <= 1
        })
        .collect()
}

fn trainer_for(id: &str) -> Option<&'static Trainer> {
    journey::TRAINERS
        .iter()
        .chain(journey::GYM_BATTLES.iter())
        .find(|t| t.id == id)
}

fn affinity_label(affinity: matterweave_monsters::roster::Affinity) -> &'static str {
    use matterweave_monsters::roster::Affinity::*;
    match affinity {
        Ember => "EMBER - aggressive attack",
        Tide => "TIDE - sturdy and flexible",
        Verdant => "VERDANT - resilient growth",
        Stone => "STONE - heavy defense",
        Gale => "GALE - swift",
        Spark => "SPARK - piercing",
        Gloom => "GLOOM - unsettling",
        Lumen => "LUMEN - radiant",
        Beast => "BEAST - balanced",
    }
}

fn describe(event: &BattleEvent, game: &Game, active: usize) -> String {
    match event {
        BattleEvent::PlayerUsedMove { name, damage } => {
            format!("Your {} used {name} for {damage}!", name_of(game, active))
        }
        BattleEvent::WildUsedMove { name, damage } => {
            format!("The wild creature used {name} for {damage}!")
        }
        BattleEvent::PlayerFainted => "Your creature fainted!".into(),
        BattleEvent::WildFainted => "The wild creature fainted!".into(),
        BattleEvent::CaptureSucceeded { .. } => "Capture succeeded!".into(),
        BattleEvent::CaptureFailed => "The creature broke free!".into(),
        BattleEvent::Escaped => "You escaped!".into(),
        BattleEvent::SwitchIn { index } => {
            format!("{} steps in!", name_of(game, *index))
        }
        BattleEvent::Heal { restored, .. } => format!("Restored {restored} HP."),
        BattleEvent::LevelUp {
            index,
            level,
            learned,
        } => match learned {
            Some(move_name) => format!(
                "{} grew to L{level} and learned {move_name}!",
                name_of(game, *index)
            ),
            None => format!("{} grew to L{level}!", name_of(game, *index)),
        },
        BattleEvent::Evolved { index, .. } => {
            format!("{} evolved!", name_of(game, *index))
        }
        BattleEvent::NoEffect { reason } => reason.clone(),
    }
}

fn name_of(game: &Game, index: usize) -> String {
    game.party
        .get(index)
        .map(|m| m.name().to_string())
        .unwrap_or_else(|| "?".into())
}

impl ApplicationHandler for MonsterApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.lifecycle.apply(PlatformEvent::WindowCreated);
        self.audio.resume();
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Mossbound")
            .with_inner_size(winit::dpi::PhysicalSize::new(1280, 720));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                log::error!("window creation failed: {error}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };
        match pollster::block_on(Renderer::new(window.clone())) {
            Ok(renderer) => {
                log::info!("Graphics: {}", renderer.capabilities);
                self.renderer = Some(renderer);
                self.window = Some(window);
                self.uploaded_revision = None;
                self.scene_key = None;
            }
            Err(error) => {
                log::error!("renderer init failed: {error}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        if self.dirty {
            self.save_now();
        }
        self.audio.suspend();
        self.lifecycle.apply(PlatformEvent::WindowDestroyed);
        self.renderer = None;
        self.window = None;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.dirty {
                    self.save_now();
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(focused) => {
                self.lifecycle.apply(if focused {
                    PlatformEvent::GainedFocus
                } else {
                    PlatformEvent::LostFocus
                });
            }
            WindowEvent::Occluded(occluded) => {
                self.lifecycle.apply(if occluded {
                    PlatformEvent::Paused
                } else {
                    PlatformEvent::Resumed
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Space) | PhysicalKey::Code(KeyCode::Enter) => {
                            if self.screen == Screen::Overworld {
                                self.interact();
                            } else {
                                self.button_action(UiAction::Confirm);
                            }
                        }
                        PhysicalKey::Code(KeyCode::Escape) => {
                            if self.screen == Screen::Menu {
                                self.button_action(UiAction::MenuClose);
                            } else {
                                self.button_action(UiAction::MenuOpen);
                            }
                        }
                        PhysicalKey::Code(KeyCode::KeyW) | PhysicalKey::Code(KeyCode::ArrowUp) => {
                            self.touch.vector = [0.0, 1.0];
                        }
                        PhysicalKey::Code(KeyCode::KeyS)
                        | PhysicalKey::Code(KeyCode::ArrowDown) => {
                            self.touch.vector = [0.0, -1.0];
                        }
                        PhysicalKey::Code(KeyCode::KeyA)
                        | PhysicalKey::Code(KeyCode::ArrowLeft) => {
                            self.touch.vector = [-1.0, 0.0];
                        }
                        PhysicalKey::Code(KeyCode::KeyD)
                        | PhysicalKey::Code(KeyCode::ArrowRight) => {
                            self.touch.vector = [1.0, 0.0];
                        }
                        _ => {}
                    }
                } else if matches!(
                    event.physical_key,
                    PhysicalKey::Code(KeyCode::KeyW)
                        | PhysicalKey::Code(KeyCode::ArrowUp)
                        | PhysicalKey::Code(KeyCode::KeyS)
                        | PhysicalKey::Code(KeyCode::ArrowDown)
                        | PhysicalKey::Code(KeyCode::KeyA)
                        | PhysicalKey::Code(KeyCode::ArrowLeft)
                        | PhysicalKey::Code(KeyCode::KeyD)
                        | PhysicalKey::Code(KeyCode::ArrowRight)
                ) {
                    self.touch.vector = [0.0, 0.0];
                }
            }
            WindowEvent::Touch(touch) => {
                let point = [touch.location.x as f32, touch.location.y as f32];
                let width = self
                    .window
                    .as_ref()
                    .map(|w| w.inner_size().width as f32)
                    .unwrap_or(1280.0);
                match touch.phase {
                    TouchPhase::Started => self.pointer_down(touch.id, point, width),
                    TouchPhase::Moved => self.pointer_move(touch.id, point),
                    TouchPhase::Ended => self.pointer_up(touch.id),
                    TouchPhase::Cancelled => {
                        self.touch.joystick = None;
                        self.touch.camera = None;
                        self.touch.vector = [0.0, 0.0];
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        if self.wants_frames() && self.renderer.is_some() {
            if let Some(window) = &self.window {
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 {
                    window.request_redraw();
                }
            }
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        if self.dirty {
            self.save_now();
        }
        self.renderer = None;
        self.window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_rotation_maps_local_plus_z_to_the_facing_direction() {
        // Yaw of PI/2 must send a +Z point toward +X, matching the formula the
        // draw path uses for the avatar.
        let (sin, cos) = (std::f32::consts::FRAC_PI_2).sin_cos();
        let z_point = [0.0f32, 0.0, 5.0];
        let rotated = [
            z_point[0] * cos + z_point[2] * sin,
            z_point[1],
            -z_point[0] * sin + z_point[2] * cos,
        ];
        assert!(rotated[0] > 4.9, "yaw rotates the model onto +X");
    }

    #[test]
    fn trainer_lookup_finds_gym_and_route_trainers() {
        assert!(trainer_for(journey::GYM_LEADER).is_some());
        assert!(trainer_for("meadow-ranger").is_some());
        assert!(trainer_for("nobody").is_none());
    }
}
