//! The incremental flora field must produce exactly the plan a from-scratch
//! call produces, at every step of a moving path and however its work is sliced.

use matterweave_core::landscape::{
    self, plan_flora, plan_flora_cached_into, FloraCellCache, FloraField, FloraPlan,
    LANDSCAPE_FLORA_TIERS,
};

const SEED: u64 = 20260913;
const MAX_SITES: usize = 9_000;
const MAX_TREES: usize = 700;

/// Ground-cover cells a full plan for the shipped tiers can cover:
/// `(2 * 72 / 2 + 1)^2` for the shipped 40/56/72 m tiers.
const GROUND_WINDOW_CELLS: usize = 73 * 73;
/// Tree cells a full plan can cover: `(2 * 160 / 8 + 1)^2`.
const TREE_WINDOW_CELLS: usize = 41 * 41;

fn fresh(eye: [f32; 3]) -> FloraPlan {
    plan_flora(SEED, eye, &LANDSCAPE_FLORA_TIERS, MAX_SITES, MAX_TREES)
}

/// A path that crosses lattice cells, tier-boundary radii, chunk edges and the
/// origin, so every eye-dependent branch of the planner runs: window bounds,
/// tier assignment and decimation, the tree radius filter and the caps.
fn path(steps: usize) -> impl Iterator<Item = [f32; 3]> {
    (0..steps).map(|step| {
        let step = step as f32;
        [
            -41.0 + step * 5.25 + (step * 0.13).sin() * 3.0,
            40.0 + (step * 0.07).cos() * 12.0,
            97.0 - step * 4.75 + (step * 0.11).sin() * 2.5,
        ]
    })
}

/// Cells one from-scratch plan generates, measured through the same memo.
fn fresh_samples(eye: [f32; 3]) -> u64 {
    let mut cache = FloraCellCache::default();
    let mut plan = FloraPlan::default();
    assert!(plan_flora_cached_into(
        SEED,
        eye,
        &LANDSCAPE_FLORA_TIERS,
        MAX_SITES,
        MAX_TREES,
        usize::MAX,
        &mut cache,
        &mut plan,
    ));
    cache.samples()
}

#[test]
fn a_moving_field_matches_a_fresh_plan_at_every_step() {
    let mut field = FloraField::new(SEED);
    for (step, eye) in path(240).enumerate() {
        assert!(
            field.advance(
                eye,
                &LANDSCAPE_FLORA_TIERS,
                MAX_SITES,
                MAX_TREES,
                usize::MAX
            ),
            "step {step} must complete with an unlimited budget"
        );
        assert_eq!(field.plan(), &fresh(eye), "step {step} at {eye:?}");
    }
}

#[test]
fn a_moving_field_covers_every_tier_and_a_tree_radius_step() {
    // The path spans more than the tree radius in one step in places; those
    // jumps must still land exactly on the fresh plan for the new eye.
    let mut field = FloraField::new(SEED);
    for eye in [
        [0.0f32, 40.0, 0.0],
        [20.0, 40.0, 0.0],
        [-500.0, 40.0, 900.0],
        [0.0, 40.0, 0.0],
        [12.0, 40.0, -12.0],
        [-9.0, 40.0, -9.0],
    ] {
        assert!(field.advance(
            eye,
            &LANDSCAPE_FLORA_TIERS,
            MAX_SITES,
            MAX_TREES,
            usize::MAX
        ));
        assert_eq!(field.plan(), &fresh(eye));
    }
}

#[test]
fn a_sliced_advance_publishes_only_a_complete_plan() {
    const BUDGET: usize = 17;
    let mut field = FloraField::new(SEED);
    for (step, eye) in path(40).enumerate() {
        let mut before = field.samples();
        let mut calls = 0usize;
        loop {
            let ready = field.advance(eye, &LANDSCAPE_FLORA_TIERS, MAX_SITES, MAX_TREES, BUDGET);
            let after = field.samples();
            assert!(
                after - before <= BUDGET as u64,
                "step {step}: one slice generated {} cells, budget is {BUDGET}",
                after - before
            );
            before = after;
            calls += 1;
            if ready {
                break;
            }
            // While incomplete the committed plan stays the previous one: the
            // next slice must not have published a partial list.
            assert!(calls < 10_000, "a {BUDGET}-cell budget must still finish");
        }
        assert!(calls > 1, "a {BUDGET}-cell budget must slice a full plan");
        assert_eq!(field.plan(), &fresh(eye), "step {step}");
    }
}

#[test]
fn retargeting_a_partial_advance_still_converges_on_the_latest_eye() {
    let mut field = FloraField::new(SEED);
    let mut last = [0.0f32; 3];
    for eye in path(80) {
        last = eye;
        // One small slice per step while the eye keeps moving: the eye is
        // retargeted before the previous plan completes, which must not corrupt
        // the memo or the committed plan.
        field.advance(eye, &LANDSCAPE_FLORA_TIERS, MAX_SITES, MAX_TREES, 64);
    }
    while !field.advance(last, &LANDSCAPE_FLORA_TIERS, MAX_SITES, MAX_TREES, 64) {}
    assert_eq!(field.plan(), &fresh(last));
}

#[test]
fn an_incomplete_advance_leaves_the_committed_plan_in_place() {
    let mut field = FloraField::new(SEED);
    let first = [10.0f32, 40.0, -10.0];
    assert!(field.advance(
        first,
        &LANDSCAPE_FLORA_TIERS,
        MAX_SITES,
        MAX_TREES,
        usize::MAX
    ));
    let committed = field.plan().clone();
    let anchor = field.anchor();
    assert!(!field.advance(
        [5000.0, 40.0, 5000.0],
        &LANDSCAPE_FLORA_TIERS,
        MAX_SITES,
        MAX_TREES,
        1
    ));
    assert_eq!(field.plan(), &committed);
    assert_eq!(field.anchor(), anchor);
}

#[test]
fn the_memo_reuses_cells_a_fresh_plan_would_regenerate() {
    let mut field = FloraField::new(SEED);
    let mut fresh_total = 0u64;
    for eye in path(120) {
        fresh_total += fresh_samples(eye);
        assert!(field.advance(
            eye,
            &LANDSCAPE_FLORA_TIERS,
            MAX_SITES,
            MAX_TREES,
            usize::MAX
        ));
    }
    let incremental = field.samples();
    assert!(
        incremental > 0,
        "the field must have generated the cells it needed"
    );
    assert!(
        incremental * 4 < fresh_total,
        "the memo generated {incremental} cells where fresh plans need {fresh_total}"
    );
    // The memo cannot grow past the windows the last plan covers.
    assert!(field.cached_ground_cells() <= GROUND_WINDOW_CELLS);
    assert!(field.cached_tree_cells() <= TREE_WINDOW_CELLS);
}

#[test]
fn caps_and_dropped_counts_match_a_fresh_plan() {
    let (max_sites, max_trees) = (137usize, 9usize);
    let mut field = FloraField::new(SEED);
    for (step, eye) in path(40).enumerate() {
        assert!(field.advance(
            eye,
            &LANDSCAPE_FLORA_TIERS,
            max_sites,
            max_trees,
            usize::MAX
        ));
        assert_eq!(
            field.plan(),
            &plan_flora(SEED, eye, &LANDSCAPE_FLORA_TIERS, max_sites, max_trees),
            "step {step}"
        );
    }
    assert!(field.plan().dropped > 0, "the caps must have refused sites");
}

#[test]
fn advancing_to_the_same_eye_is_free_and_not_a_new_generation() {
    let mut field = FloraField::new(SEED);
    let eye = [3.25f32, 44.0, -7.5];
    assert!(field.advance(
        eye,
        &LANDSCAPE_FLORA_TIERS,
        MAX_SITES,
        MAX_TREES,
        usize::MAX
    ));
    let generation = field.generation();
    let samples = field.samples();
    assert!(field.advance(eye, &LANDSCAPE_FLORA_TIERS, MAX_SITES, MAX_TREES, 1));
    assert_eq!(field.generation(), generation);
    assert_eq!(field.samples(), samples);
}

#[test]
fn nonfinite_eyes_plan_like_the_origin() {
    let mut field = FloraField::new(SEED);
    for eye in [
        [f32::NAN, 0.0, f32::NAN],
        [f32::INFINITY, 0.0, f32::NEG_INFINITY],
    ] {
        assert!(field.advance(
            eye,
            &LANDSCAPE_FLORA_TIERS,
            MAX_SITES,
            MAX_TREES,
            usize::MAX
        ));
        assert_eq!(field.plan(), &fresh([0.0; 3]));
        assert_eq!(field.anchor(), Some([0, 0]));
    }
    assert_eq!(
        plan_flora(SEED, [0.0; 3], &LANDSCAPE_FLORA_TIERS, MAX_SITES, MAX_TREES),
        fresh([f32::NAN, 0.0, 0.0])
    );
}

#[test]
fn the_shipped_tier_set_is_the_one_the_window_bounds_assume() {
    // The constants above document the memo bound; if the tiers change, the
    // bound must be recomputed rather than silently wrong.
    let outer = landscape::LANDSCAPE_FLORA_TIERS
        .iter()
        .map(|tier| tier.radius_m)
        .max()
        .unwrap();
    let cells = (2 * outer / landscape::FLORA_CELL_M + 1) as usize;
    assert_eq!(cells * cells, GROUND_WINDOW_CELLS);
    let tree_cells = (2 * landscape::LANDSCAPE_TREE_RADIUS_M / landscape::TREE_CELL_M + 1) as usize;
    assert_eq!(tree_cells * tree_cells, TREE_WINDOW_CELLS);
}
