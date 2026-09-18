//! End-to-end loop test: new game through capture, evolution, a trainer, the
//! gym finale and save/resume. This exercises the same calls the app makes,
//! without a window, so the rules path is pinned independently of rendering.

use matterweave_monsters::battle::{resolve, Battle, BattleAction, BattlePhase, Rolls};
use matterweave_monsters::journey::{self, Place};
use matterweave_monsters::monsters::Monster;
use matterweave_monsters::roster::{ids, SpeciesId};
use matterweave_monsters::state::{Game, SaveFile};

/// Fight until the battle resolves, capturing first when the target is weak.
fn auto_battle(
    game: &mut Game,
    battle: &mut Battle,
    active: usize,
    rolls: &mut Rolls,
    capture: bool,
) {
    let mut guard = 0;
    while battle.phase == BattlePhase::PlayerChoice && guard < 40 {
        let action = if capture && battle.wild.current_hp * 3 < battle.wild.max_hp() {
            BattleAction::Capture
        } else {
            // Prefer the first move with PP left.
            let index = game.party[active]
                .moves
                .iter()
                .position(|slot| slot.current_pp > 0)
                .unwrap_or(0);
            BattleAction::Fight(index)
        };
        resolve(game, battle, active, action, rolls).expect("legal action");
        guard += 1;
    }
}

#[test]
fn full_journey_from_starter_to_badge_and_resume() {
    let mut rolls = Rolls::new(2026);
    let mut game = Game::new_game("Wren", ids::CINDERUB).expect("starter");

    // Meet and capture a wild creature on the first route.
    let wild = Monster::wild(ids::NIBBIT, 4).unwrap();
    let mut battle = Battle::wild_encounter(wild, game.party[0].level);
    auto_battle(&mut game, &mut battle, 0, &mut rolls, true);
    assert!(
        matches!(battle.phase, BattlePhase::Captured | BattlePhase::Won),
        "the encounter resolves, got {:?}",
        battle.phase
    );
    if battle.phase == BattlePhase::Captured {
        assert_eq!(game.party.len(), 2, "the wild creature joined");
    }

    // Grind safely: level the starter through wild battles until it evolves.
    let mut guard = 0;
    while game.party[0].species == ids::CINDERUB && guard < 400 {
        let target = Monster::wild(ids::NIBBIT, 3).unwrap();
        let mut battle = Battle::wild_encounter(target, game.party[0].level);
        if game.party[0].current_hp * 4 < game.party[0].max_hp() {
            game.haven_heal();
        }
        auto_battle(&mut game, &mut battle, 0, &mut rolls, false);
        guard += 1;
    }
    assert_eq!(
        game.party[0].species,
        ids::EMBERPACK,
        "Cinderub evolved at 16"
    );
    assert!(game.party[0].level >= 16);
    assert!(
        game.party[0].level < matterweave_monsters::monsters::MAX_LEVEL,
        "the demo does not require max level"
    );

    // Beat a route trainer for supplies.
    let trainer = journey::TRAINERS
        .iter()
        .find(|t| t.id == "meadow-ranger")
        .unwrap();
    let mut queue: Vec<Monster> = trainer
        .party
        .iter()
        .map(|m| Monster::wild(m.species, m.level).unwrap())
        .collect();
    let mut sent = 0;
    while let Some(next) = queue.first().cloned() {
        queue.remove(0);
        let mut battle = Battle::wild_encounter(next, game.party[0].level);
        auto_battle(&mut game, &mut battle, 0, &mut rolls, false);
        assert_ne!(battle.phase, BattlePhase::Lost, "the trained starter wins");
        sent += 1;
        if game.party[0].current_hp * 4 < game.party[0].max_hp() {
            game.haven_heal();
        }
    }
    assert_eq!(sent, trainer.party.len());
    game.defeated_trainers.push(trainer.id.to_string());
    game.restock(trainer.reward_charms, trainer.reward_tonics);

    // The gym: lead-in attendants, then the leader. The party heals between
    // fights through the haven, as ordinary play allows.
    let mut gym_order: Vec<&journey::Trainer> = journey::GYM_BATTLES.iter().collect();
    let leader = gym_order.pop().expect("a leader exists");
    for attendant in gym_order {
        for picked in attendant.party {
            let mut battle = Battle::wild_encounter(
                Monster::wild(picked.species, picked.level).unwrap(),
                game.party[0].level,
            );
            game.haven_heal();
            auto_battle(&mut game, &mut battle, 0, &mut rolls, false);
            assert_ne!(
                battle.phase,
                BattlePhase::Lost,
                "{} is beatable",
                attendant.title
            );
        }
        game.defeated_trainers.push(attendant.id.to_string());
        game.restock(attendant.reward_charms, attendant.reward_tonics);
    }
    for picked in leader.party {
        let mut battle = Battle::wild_encounter(
            Monster::wild(picked.species, picked.level).unwrap(),
            game.party[0].level,
        );
        game.haven_heal();
        auto_battle(&mut game, &mut battle, 0, &mut rolls, false);
        assert_ne!(
            battle.phase,
            BattlePhase::Lost,
            "the leader's team is beatable"
        );
    }
    game.defeated_trainers.push(leader.id.to_string());
    game.restock(leader.reward_charms, leader.reward_tonics);
    game.gym_badge = true;

    // Save at the gym and resume with everything intact.
    let dir = std::env::temp_dir().join(format!("mossbound-loop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("save.json");
    let mut save = SaveFile::new(game, Place::Tidewater);
    save.position = [16.5, 2.0, 12.5];
    save.save(&path).expect("save writes");
    let resumed = SaveFile::load(&path).expect("save loads");
    assert!(resumed.game.gym_badge);
    assert_eq!(resumed.game.party[0].species, ids::EMBERPACK);
    assert!(resumed
        .game
        .defeated_trainers
        .iter()
        .any(|t| t == journey::GYM_LEADER));
    assert!(resumed.game.capture_charms > 0, "supplies remain usable");
}

#[test]
fn defeat_recovers_at_the_haven_without_losing_monsters() {
    let mut rolls = Rolls::new(11);
    let mut game = Game::new_game("Wren", ids::BROOKLET).expect("starter");
    let before = game.party.len();
    // A hopeless fight against a much stronger target.
    let mut battle = Battle::wild_encounter(Monster::wild(ids::TERRAFANG, 38).unwrap(), 5);
    auto_battle(&mut game, &mut battle, 0, &mut rolls, false);
    assert_eq!(battle.phase, BattlePhase::Lost);
    // The app's whiteout path: full heal and return to the starting town.
    game.haven_heal();
    assert_eq!(game.party.len(), before, "defeat never deletes a creature");
    assert!(game.party[0].current_hp > 0, "the haven revives the team");
}

#[test]
fn the_two_unchosen_starter_lines_are_reachable_after_the_badge() {
    // A Brooklet save can still obtain Cinderub and Sproutlet through the
    // post-gym Quarry Loop sanctuary.
    let route = journey::route(Place::QuarryLoop).expect("the loop exists");
    let post: Vec<SpeciesId> = route
        .encounters
        .iter()
        .filter(|e| e.post_gym)
        .map(|e| e.species)
        .collect();
    // Every starter base line is in the sanctuary, so whichever the player
    // chose, the other two are reachable in the same save.
    for id in matterweave_monsters::roster::STARTERS {
        assert!(
            post.contains(&id),
            "starter line {id:?} is reachable post-gym"
        );
    }
}
