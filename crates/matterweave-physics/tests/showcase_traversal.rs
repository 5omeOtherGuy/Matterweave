//! Full source collision and continuous walking; opt in to this campaign gate.
//! Initial placement mirrors the app's bounded capsule validation. Subsequent
//! movement uses only normal Physics::step, including the app's wading speed.
//! Reported duration is simulated traversal, never physical phone evidence.
use matterweave_detail::{build_showcase, EYE_HEIGHT_M, SHOWCASE_SEED, WALK_SPEED_M_S};
use matterweave_physics::{Physics, FIXED_DT};
use std::time::{Duration, Instant};

fn traverse(elevated: bool) {
    let map = build_showcase(SHOWCASE_SEED).unwrap();
    let route = if elevated {
        &map.elevated_route
    } else {
        &map.route
    };
    let waterside_last = map.waterside_route().len().saturating_sub(1);
    let mut physics = Physics::new(&matterweave_core::World::new(SHOWCASE_SEED));
    physics.replace_detail_scene(&map.scene).unwrap();
    let start = route[0];
    assert!(
        (0..=8).any(|lift| physics.teleport([
            start[0],
            start[1] + EYE_HEIGHT_M + lift as f32 * 0.125,
            start[2]
        ])),
        "route entrance rejected: {start:?}"
    );
    for _ in 0..120 {
        physics.step(FIXED_DT, [0.; 3], false);
    }
    assert!(physics.grounded(), "route entrance has no floor");
    let wall = Instant::now();
    let mut steps = 0;
    for (index, target) in route.iter().enumerate().skip(1) {
        let began = steps;
        loop {
            let eye = physics.character_eye();
            let dx = target[0] - eye[0];
            let dz = target[2] - eye[2];
            let distance = dx.hypot(dz);
            if distance < 0.2 {
                break;
            }
            assert!(
                steps - began < 1200,
                "stalled elevated={elevated} point={index} target={target:?} eye={eye:?} sim_s={}",
                steps as f32 * FIXED_DT
            );
            assert!(
                eye[1] > target[1] - 4.,
                "fell below route at {index}: {eye:?} target={target:?}"
            );
            assert!(
                wall.elapsed() < Duration::from_secs(180),
                "host traversal wall limit at {index}"
            );
            let depth = map
                .terrain
                .water_surface_at_metres(eye[0], eye[2])
                .map_or(0., |s| s - (eye[1] - 1.5));
            let speed = if depth > 0.2 { 2. } else { WALK_SPEED_M_S };
            physics.step(
                FIXED_DT,
                [dx / distance * speed, 0., dz / distance * speed],
                false,
            );
            steps += 1;
        }
        if !elevated && index == waterside_last {
            eprintln!(
                "WATERSIDE TRAVERSAL PASS points={} sim_seconds={}",
                index + 1,
                steps as f32 * FIXED_DT
            );
        }
    }
    let seconds = steps as f32 * FIXED_DT;
    eprintln!(
        "TRAVERSAL PASS elevated={elevated} points={} sim_seconds={seconds} host_seconds={:.3}",
        route.len(),
        wall.elapsed().as_secs_f64()
    );
    if !elevated {
        assert!(
            (180.0..=300.0).contains(&seconds),
            "ground loop duration {seconds}"
        );
    }
}

#[test]
#[ignore = "full scene campaign gate; real continuous character simulation"]
fn ground_loop_is_walkable() {
    traverse(false);
}

#[test]
#[ignore = "full scene campaign gate; separate validated elevated entrance"]
fn elevated_route_is_walkable() {
    traverse(true);
}
