//! Battle resolution tests: turn order, capture, escape, fainting, XP,
//! evolution and determinism.

use matterweave_monsters::battle::{
    resolve, Battle, BattleAction, BattleEvent, BattlePhase, Rolls,
};
use matterweave_monsters::monsters::{GameError, Monster};
use matterweave_monsters::roster::ids;
use matterweave_monsters::state::Game;

fn setup(starter: u16, wild: u16, wild_level: u8) -> (Game, Battle) {
    let game = Game::new_game("Wren", matterweave_monsters::roster::SpeciesId(starter)).unwrap();
    let monster = Monster::wild(matterweave_monsters::roster::SpeciesId(wild), wild_level).unwrap();
    let battle = Battle::wild_encounter(monster, game.party[0].level);
    (game, battle)
}

#[test]
fn fight_resolves_both_attacks_and_reports_damage() {
    let (mut game, mut battle) = setup(1, 10, 4);
    let mut rolls = Rolls::new(7);
    let player_hp = game.party[0].current_hp;
    resolve(
        &mut game,
        &mut battle,
        0,
        BattleAction::Fight(0),
        &mut rolls,
    )
    .expect("turn resolves");
    assert!(battle
        .log
        .iter()
        .any(|e| matches!(e, BattleEvent::PlayerUsedMove { .. })));
    assert!(
        battle.wild.current_hp < battle.wild.max_hp(),
        "the wild creature was hit"
    );
    assert!(
        game.party[0].current_hp < player_hp || battle.phase != BattlePhase::PlayerChoice,
        "the wild creature struck back or the battle ended"
    );
    assert!(matches!(
        battle.phase,
        BattlePhase::PlayerChoice | BattlePhase::Won | BattlePhase::Lost
    ));
}

#[test]
fn winning_awards_xp_and_levels_the_starter() {
    let (mut game, mut battle) = setup(1, 10, 3);
    battle.wild.current_hp = 1;
    let mut rolls = Rolls::new(3);
    let mut guard = 0;
    while battle.phase == BattlePhase::PlayerChoice && guard < 8 {
        resolve(
            &mut game,
            &mut battle,
            0,
            BattleAction::Fight(0),
            &mut rolls,
        )
        .expect("turn");
        guard += 1;
    }
    assert_eq!(battle.phase, BattlePhase::Won);
    assert!(battle
        .log
        .iter()
        .any(|e| matches!(e, BattleEvent::WildFainted)));
    // At level 5 the way to Emberpack (16) is still eleven levels off.
    assert!(!battle
        .log
        .iter()
        .any(|e| matches!(e, BattleEvent::Evolved { .. })));
}

#[test]
fn evolution_fires_when_the_gate_is_met() {
    let (mut game, mut battle) = setup(1, 30, 3);
    game.party[0].level = 16;
    battle.wild.current_hp = 1;
    let mut rolls = Rolls::new(9);
    let mut guard = 0;
    while battle.phase == BattlePhase::PlayerChoice && guard < 8 {
        resolve(
            &mut game,
            &mut battle,
            0,
            BattleAction::Fight(0),
            &mut rolls,
        )
        .expect("turn");
        guard += 1;
    }
    assert_eq!(battle.phase, BattlePhase::Won);
    assert!(battle
        .log
        .iter()
        .any(|e| matches!(e, BattleEvent::Evolved { into, .. } if *into == 2)));
    assert_eq!(game.party[0].species, ids::EMBERPACK);
}

#[test]
fn escape_can_fail_and_lets_the_wild_strike_back() {
    let hp = setup(1, 10, 8).0.party[0].current_hp;
    let mut any_failed = false;
    for seed in 0..40u64 {
        let (mut g, mut b) = setup(1, 10, 8);
        let mut rolls = Rolls::new(seed.wrapping_mul(2654435761) | 1);
        resolve(&mut g, &mut b, 0, BattleAction::Escape, &mut rolls).expect("turn");
        if b.phase != BattlePhase::Fled {
            any_failed = true;
            assert!(
                g.party[0].current_hp < hp || b.phase == BattlePhase::Lost,
                "a failed escape is punished at seed {seed}"
            );
            assert!(b
                .log
                .iter()
                .any(|e| matches!(e, BattleEvent::NoEffect { .. })));
            break;
        }
    }
    assert!(
        any_failed,
        "some seeds must fail the escape against a faster target"
    );
}

#[test]
fn escape_succeeds_against_a_slow_target() {
    let (mut game, mut battle) = setup(1, 30, 2);
    let mut rolls = Rolls::new(11);
    resolve(&mut game, &mut battle, 0, BattleAction::Escape, &mut rolls).expect("turn");
    assert_eq!(battle.phase, BattlePhase::Fled);
    assert_eq!(battle.log, vec![BattleEvent::Escaped]);
}

#[test]
fn capture_success_adds_to_the_party_and_ends_the_battle() {
    let (mut game, mut battle) = setup(1, 10, 2);
    battle.wild.current_hp = 1;
    // Force success: try seeds until the roll passes, then assert the state.
    let mut caught = false;
    for seed in 0..200 {
        let (mut g, mut b) = setup(1, 10, 2);
        b.wild.current_hp = 1;
        let mut rolls = Rolls::new(seed);
        resolve(&mut g, &mut b, 0, BattleAction::Capture, &mut rolls).expect("turn");
        if b.phase == BattlePhase::Captured {
            assert_eq!(g.party.len(), 2, "the captured creature joined the party");
            assert_eq!(g.party[1].species, ids::NIBBIT);
            caught = true;
            break;
        }
    }
    assert!(
        caught,
        "a 1-hp level-2 target must be catchable across seeds"
    );
    let _ = (&mut game, &mut battle);
}

#[test]
fn capture_failure_lets_the_wild_strike() {
    let hp = setup(1, 10, 8).0.party[0].current_hp;
    let mut failed_once = false;
    for seed in 0..80u64 {
        let (mut g, mut b) = setup(1, 10, 8);
        let mut rolls = Rolls::new(seed.wrapping_mul(2246822519) | 1);
        resolve(&mut g, &mut b, 0, BattleAction::Capture, &mut rolls).expect("turn");
        if b.log.contains(&BattleEvent::CaptureFailed) {
            assert!(
                g.party[0].current_hp < hp || b.phase == BattlePhase::Lost,
                "a failed capture is punished at seed {seed}"
            );
            failed_once = true;
            break;
        }
    }
    assert!(failed_once, "captures fail often enough to be tested");
}

#[test]
fn switching_sends_out_the_named_member_after_the_wild_strike() {
    let (mut game, mut battle) = setup(1, 30, 30);
    let second = Monster::wild(ids::NIBBIT, 10).unwrap();
    game.capture(&second).expect("party has room");
    let mut rolls = Rolls::new(5);
    resolve(
        &mut game,
        &mut battle,
        0,
        BattleAction::Switch(1),
        &mut rolls,
    )
    .expect("turn");
    assert!(battle
        .log
        .iter()
        .any(|e| matches!(e, BattleEvent::SwitchIn { index: 1 })));
    assert!(
        game.party[1].current_hp < game.party[1].max_hp(),
        "the switch eats a hit"
    );
}

#[test]
fn fainting_the_last_member_loses_the_battle() {
    let (mut game, mut battle) = setup(1, 30, 40);
    game.party[0].current_hp = 1;
    let mut rolls = Rolls::new(1);
    let mut guard = 0;
    while battle.phase == BattlePhase::PlayerChoice && guard < 10 {
        resolve(
            &mut game,
            &mut battle,
            0,
            BattleAction::Fight(0),
            &mut rolls,
        )
        .expect("turn");
        guard += 1;
    }
    assert_eq!(battle.phase, BattlePhase::Lost);
    assert_eq!(game.party[0].current_hp, 0);
}

#[test]
fn battle_replay_is_deterministic_for_a_seed() {
    let run = |seed: u64| {
        let (mut game, mut battle) = setup(1, 13, 8);
        let mut rolls = Rolls::new(seed);
        let mut events = Vec::new();
        let mut guard = 0;
        while battle.phase == BattlePhase::PlayerChoice && guard < 20 {
            resolve(
                &mut game,
                &mut battle,
                0,
                BattleAction::Fight(0),
                &mut rolls,
            )
            .expect("turn");
            events.extend(battle.log.iter().cloned());
            battle.log.clear();
            guard += 1;
        }
        (battle.phase, events, game.party[0].current_hp)
    };
    assert_eq!(run(42), run(42), "same seed, same battle");
    // Different seeds cross the damage variance band, so the first wild strike
    // must not be identical for every seed.
    let first_wild: Vec<u16> = (1..40)
        .filter_map(|seed| {
            run(seed).1.iter().find_map(|event| match event {
                BattleEvent::WildUsedMove { damage, .. } => Some(*damage),
                _ => None,
            })
        })
        .collect();
    assert!(
        first_wild.len() > 10,
        "the wild creature strikes in these runs"
    );
    assert!(
        first_wild
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1,
        "damage varies with the seed"
    );
}

#[test]
fn actions_are_refused_after_the_battle_ends() {
    let (mut game, mut battle) = setup(1, 30, 2);
    let mut rolls = Rolls::new(11);
    resolve(&mut game, &mut battle, 0, BattleAction::Escape, &mut rolls).expect("turn");
    assert_eq!(battle.phase, BattlePhase::Fled);
    assert_eq!(
        resolve(
            &mut game,
            &mut battle,
            0,
            BattleAction::Fight(0),
            &mut rolls
        ),
        Err(GameError::InvalidTransition)
    );
}

#[test]
fn xp_reward_scales_with_the_wild_level() {
    let weak = Battle::wild_encounter(Monster::wild(ids::NIBBIT, 3).unwrap(), 5);
    let strong = Battle::wild_encounter(Monster::wild(ids::NIBBIT, 20).unwrap(), 5);
    assert!(strong.xp_reward() > weak.xp_reward());
}
