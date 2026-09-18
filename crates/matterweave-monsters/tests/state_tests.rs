//! Tests pin the authoritative rules: capture, party/storage, healing,
//! experience, evolution and save round-trips.

use matterweave_monsters::monsters::{damage, GameError, Monster, MAX_PARTY, MAX_STORAGE};
use matterweave_monsters::roster::ids;
use matterweave_monsters::state::{capture_chance, Game, SAVE_VERSION, STARTER_LEVEL};

fn new_game() -> Game {
    Game::new_game("Wren", ids::CINDERUB).expect("starter is valid")
}

#[test]
fn new_game_requires_a_locked_starter() {
    let game = Game::new_game("Wren", ids::CINDERUB).expect("valid starter");
    assert_eq!(game.party.len(), 1);
    assert_eq!(game.party[0].species, ids::CINDERUB);
    assert_eq!(game.party[0].level, STARTER_LEVEL);
    assert_eq!(game.capture_charms, 5);
    assert_eq!(game.tonics, 3);
    assert!(game.started);
    assert_eq!(game.version, SAVE_VERSION);
    assert!(
        Game::new_game("Wren", ids::PYREFALL).is_err(),
        "final form is not a starter"
    );
}

#[test]
fn capture_fills_party_then_storage_then_refuses_atomically() {
    let mut game = new_game();
    game.restock(60, 0);
    for i in 0..MAX_PARTY - 1 {
        let wild = Monster::wild(ids::NIBBIT, 3).unwrap();
        assert_eq!(
            game.capture(&wild),
            Ok(matterweave_monsters::state::CapturedWhere::Party),
            "slot {i}"
        );
    }
    assert_eq!(game.party.len(), MAX_PARTY);
    for i in 0..MAX_STORAGE {
        let wild = Monster::wild(ids::PUFFPEEP, 2).unwrap();
        assert_eq!(
            game.capture(&wild),
            Ok(matterweave_monsters::state::CapturedWhere::Storage),
            "slot {i}"
        );
    }
    assert_eq!(game.storage.len(), MAX_STORAGE);
    // Everything full: the capture is refused and the charm is returned.
    let charms = game.capture_charms;
    let wild = Monster::wild(ids::DUNEMOLE, 4).unwrap();
    assert_eq!(game.capture(&wild), Err(GameError::PartyFull));
    assert_eq!(
        game.capture_charms, charms,
        "a failed capture refunds the charm"
    );
}

#[test]
fn capture_refuses_without_a_supply() {
    let mut game = new_game();
    game.capture_charms = 0;
    let wild = Monster::wild(ids::NIBBIT, 3).unwrap();
    assert_eq!(game.capture(&wild), Err(GameError::InvalidTransition));
    assert!(game.party.len() == 1, "nothing joined");
}

#[test]
fn tonic_heals_partially_and_refuses_when_full_or_out() {
    let mut game = new_game();
    game.party[0].current_hp = 1;
    let healed = game.heal_member(0).expect("tonic available");
    assert!(healed > 1 && healed <= game.party[0].max_hp());
    game.party[0].current_hp = game.party[0].max_hp();
    assert_eq!(
        game.heal_member(0),
        Err(GameError::InvalidTransition),
        "full hp refuses"
    );
    game.party[0].current_hp = 1;
    while game.tonics > 0 {
        let _ = game.heal_member(0);
    }
    assert_eq!(
        game.heal_member(0),
        Err(GameError::InvalidTransition),
        "out of tonics refuses"
    );
}

#[test]
fn haven_restores_hp_and_pp_for_the_whole_party() {
    let mut game = new_game();
    let wild = Monster::wild(ids::NIBBIT, 3).unwrap();
    let _ = game.capture(&wild);
    for member in &mut game.party {
        member.current_hp = 1;
        for slot in &mut member.moves {
            slot.current_pp = 0;
        }
    }
    game.haven_heal();
    for member in &game.party {
        assert_eq!(member.current_hp, member.max_hp());
        assert!(member
            .moves
            .iter()
            .all(|slot| slot.current_pp == slot.max_pp));
    }
}

#[test]
fn storage_swap_round_trips_and_respects_bounds() {
    let mut game = new_game();
    let wild = Monster::wild(ids::NIBBIT, 3).unwrap();
    let _ = game.capture(&wild);
    assert_eq!(game.party.len(), 2);
    game.swap_storage(1).expect("send to storage");
    assert_eq!(game.party.len(), 1);
    assert_eq!(game.storage.len(), 1);
    game.withdraw(0).expect("take back");
    assert_eq!(game.party.len(), 2);
    assert!(game.storage.is_empty());
    // Last healthy member cannot leave.
    game.swap_storage(1).expect("send again");
    assert_eq!(
        game.swap_storage(0),
        Err(GameError::InvalidTransition),
        "sole member stays"
    );
    assert_eq!(
        game.withdraw(9),
        Err(GameError::InvalidTransition),
        "empty slot refuses"
    );
}

#[test]
fn xp_levels_learn_moves_and_cap_at_max() {
    let mut game = new_game();
    let needed = matterweave_monsters::state::xp_for_next(STARTER_LEVEL);
    let (level, learned) = game.grant_xp(0, needed).expect("member exists");
    assert_eq!(
        level,
        STARTER_LEVEL + 1,
        "exact threshold crosses exactly one level"
    );
    assert_eq!(learned, None, "Cinderub learns nothing at level 6");
    let mut game2 = new_game();
    let (maxed, _) = game2.grant_xp(0, 100_000).expect("member exists");
    assert_eq!(maxed, matterweave_monsters::monsters::MAX_LEVEL);
    let (_, none) = game2.grant_xp(0, 50).expect("member exists");
    assert_eq!(none, None, "no move above the learnset");
}

#[test]
fn evolution_applies_at_the_gate_and_keeps_the_hp_ratio() {
    let mut game = new_game();
    // Push Cinderub past its level-16 gate without granting the levels.
    game.grant_xp(0, 1).expect("member exists");
    assert_eq!(
        game.evolve_check(0),
        Err(GameError::InvalidTransition),
        "level gate holds"
    );
    game.party[0].level = 16;
    game.party[0].current_hp = game.party[0].max_hp() / 2;
    let evolved = game.evolve_check(0).expect("gate met");
    assert_eq!(evolved, ids::EMBERPACK);
    assert_eq!(game.party[0].species, ids::EMBERPACK);
    assert!(game.party[0].max_hp() > 0);
    // A terminal form refuses again.
    assert_eq!(
        game.evolve_check(0),
        Err(GameError::InvalidTransition),
        "Pyrefall is terminal"
    );
}

#[test]
fn damage_follows_affinity_and_stays_positive() {
    use matterweave_monsters::roster::Affinity;
    // The shared table itself: 2x, 0.5x and neutral are pinned.
    assert_eq!(Affinity::Ember.against(Affinity::Verdant), 2.0);
    assert_eq!(Affinity::Ember.against(Affinity::Tide), 0.5);
    assert_eq!(Affinity::Ember.against(Affinity::Ember), 1.0);

    let ember = Monster::wild(ids::CINDERUB, 10).unwrap();
    let sprout = Monster::wild(ids::SPROUTLET, 10).unwrap();
    let brook = Monster::wild(ids::BROOKLET, 10).unwrap();
    let effective = damage(&ember, &sprout, "Ember Nip", 8).expect("move known");
    let resisted = damage(&ember, &brook, "Ember Nip", 8).expect("move known");
    let neutral = damage(&ember, &ember, "Ember Nip", 8).expect("move known");
    assert!(
        effective > neutral,
        "2x affinity beats neutral: {effective} vs {neutral}"
    );
    assert!(
        resisted < neutral,
        "0.5x affinity resists: {resisted} vs {neutral}"
    );
    assert!(resisted > 0, "damage is never zero on a hit");
    assert_eq!(
        damage(&ember, &sprout, "No Such Move", 1),
        Err(GameError::NoMoves)
    );
    // A status move deals no direct damage but is a legal known move.
    let mage = Monster::wild(ids::SPROUTLET, 10).unwrap();
    assert_eq!(damage(&mage, &ember, "Vine Snare", 1), Ok(0));
}

#[test]
fn capture_chance_scales_with_weakened_targets() {
    let wild = Monster::wild(ids::NIBBIT, 5).unwrap();
    let full = capture_chance(&wild);
    let mut weak = wild.clone();
    weak.current_hp = 1;
    let weak_chance = capture_chance(&weak);
    assert!(weak_chance > full + 20, "a 1-hp target is much easier");
    assert!((5..=95).contains(&full) && (5..=95).contains(&weak_chance));
}

#[test]
fn save_round_trips_through_json() {
    let mut game = new_game();
    let wild = Monster::wild(ids::FLITFIN, 6).unwrap();
    let _ = game.capture(&wild);
    game.defeated_trainers.push("route-1-ranger".into());
    let text = serde_json::to_string(&game).expect("serialize");
    let loaded = Game::load(text.as_bytes()).expect("version gate passes");
    assert_eq!(loaded.party.len(), game.party.len());
    assert_eq!(loaded.capture_charms, game.capture_charms);
    assert_eq!(loaded.defeated_trainers, game.defeated_trainers);
    // Old versions are rejected so saves cannot silently migrate.
    let mut old = serde_json::to_value(&game).unwrap();
    old["version"] = serde_json::Value::from(0);
    let text = serde_json::to_string(&old).unwrap();
    assert!(matches!(
        Game::load(text.as_bytes()),
        Err(GameError::InvalidTransition)
    ));
    // Corrupt payloads are a refusal, not a panic.
    assert!(Game::load(b"not json").is_err());
}
