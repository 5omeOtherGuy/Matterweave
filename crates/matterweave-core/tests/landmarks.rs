//! Landmark acceptance: the demo world's three biomes, its landmarks, the
//! framing of the spawn view, walkable footprints and the walk that reaches one.
//!
//! Every number here is derived from the generator and the named spawn
//! ([`landmarks::DEMO_SPAWN`]), so a change that moves the demo fails a test
//! instead of quietly shipping a different world.
use matterweave_core::landmarks::{self, Landmark, LandmarkKind};
use matterweave_core::landscape::{self, MAX_SURFACE_Y};
use matterweave_core::material;
use std::collections::{BTreeMap, VecDeque};

const SEED: u64 = landmarks::DEMO_SEED;
const SPAWN: [i32; 3] = landmarks::DEMO_SPAWN;
const YAW: f64 = landmarks::DEMO_YAW_DEGREES as f64;
/// Walking speed the framing argument is made at, in metres per second.
const WALK_SPEED_MPS: f32 = 4.5;
/// A few minutes at that speed; the radius three biomes must be inside.
const WALK_RADIUS_M: i32 = 600;
/// Ground a biome needs inside the radius before it is a place and not a patch.
const BIOME_AREA_M2: i64 = 40_000;
/// Landscape sample's vertical field of view and reference viewport height: the
/// projection in `apps/explorer` is 65 degrees, and the derivation in
/// `landscape::LOD_STEP_M` is written for a 1440-pixel-tall landscape viewport.
const VIEW_FOV_DEGREES: f64 = 65.0;
const VIEWPORT_PIXELS: f64 = 1440.0;
/// Half-angle of the view wedge a landmark has to be inside: half of the 97
/// degree horizontal field a 16:9 frame has at a 65 degree vertical field.
const WEDGE_HALF_ANGLE_DEGREES: f64 = 48.0;

fn eye() -> [i32; 3] {
    [
        SPAWN[0],
        SPAWN[1] + landmarks::DEMO_EYE_ABOVE_GROUND_M,
        SPAWN[2],
    ]
}

/// Bearing of a target from the spawn, in degrees from +Z.
fn bearing_to(target: [i32; 2]) -> f64 {
    let dx = (target[0] - SPAWN[0]) as f64;
    let dz = (target[1] - SPAWN[2]) as f64;
    dx.atan2(dz).to_degrees().rem_euclid(360.0)
}

/// Signed angle between the spawn's facing and a target, in degrees.
fn angle_off_axis(target: [i32; 2]) -> f64 {
    let mut difference = bearing_to(target) - YAW;
    while difference > 180.0 {
        difference -= 360.0;
    }
    while difference < -180.0 {
        difference += 360.0;
    }
    difference
}

/// Whether the straight line from an eye to a summit clears the terrain.
///
/// Sampled every 12 m with the generator's own shaped heights, so a landmark
/// that another landmark or a ridge hides is not counted as visible. Trees are
/// not sampled: this is terrain visibility, and the capture is what shows
/// whether a canopy is in the way.
fn summit_is_visible(eye: [i32; 3], summit: [i32; 3]) -> bool {
    let dx = summit[0] - eye[0];
    let dz = summit[2] - eye[2];
    let distance = ((dx as i64 * dx as i64 + dz as i64 * dz as i64) as u64).isqrt() as i32;
    if distance < 50 {
        return false;
    }
    for step in 1..(distance / 12) {
        let t = step * 12;
        let x = eye[0] + dx * t / distance;
        let z = eye[2] + dz * t / distance;
        let ray_y = eye[1] + (summit[1] - eye[1]) * t / distance;
        if landscape::height_at(SEED, x, z) > ray_y {
            return false;
        }
    }
    true
}

/// Pixels a `height`-metre silhouette at `distance` metres subtends in a
/// reference viewport.
fn silhouette_pixels(height_m: i32, distance_m: i32) -> f64 {
    let half = (VIEW_FOV_DEGREES / 2.0).to_radians().tan();
    VIEWPORT_PIXELS * (height_m as f64 / distance_m as f64) / (2.0 * half)
}

/// How much of the landmark a player can actually see, in metres of its own
/// height: the part of its silhouette that stands above whatever ridge is in the
/// way.
///
/// The terrain along the eye-to-summit line is compared by *angle*, not by
/// height: a ridge 40 m below the sight line at 100 m hides more of a landmark
/// than the same ridge at a kilometre does. A landmark whose summit only just
/// clears a hill is one a player cannot pick out, and this is that measurement.
fn visible_height_m(eye: [i32; 3], landmark: &Landmark) -> i32 {
    let summit = summit(landmark);
    let dx = summit[0] - eye[0];
    let dz = summit[2] - eye[2];
    let distance = ((dx as i64 * dx as i64 + dz as i64 * dz as i64) as u64).isqrt() as i32;
    if distance < 50 {
        return 0;
    }
    // Only the ground in *front* of the landmark counts: the ray to the summit
    // crosses the landmark's own flank, and reading that as the skyline would
    // report a tower as hidden behind itself.
    let front = (distance - landmark.radius_m()).max(8);
    let mut skyline = f64::NEG_INFINITY;
    for step in 1..(front / 8) {
        let t = (step * 8) as f64;
        let x = eye[0] + dx * step * 8 / distance;
        let z = eye[2] + dz * step * 8 / distance;
        skyline = skyline.max((landscape::height_at(SEED, x, z) - eye[1]) as f64 / t);
    }
    let target = (summit[1] - eye[1]) as f64 / distance as f64;
    ((target - skyline) * distance as f64).max(0.0) as i32
}

/// The landmark's summit, from the generator.
fn summit(landmark: &Landmark) -> [i32; 3] {
    [
        landmark.centre[0],
        landscape::height_at(SEED, landmark.centre[0], landmark.centre[1]),
        landmark.centre[1],
    ]
}

fn landmarks_within(range: i32) -> Vec<Landmark> {
    landmarks::sites_around(SEED, [SPAWN[0], SPAWN[2]], range)
}

#[test]
fn the_demo_spawn_is_settled_ground_with_a_view() {
    let column = landscape::column(SEED, SPAWN[0], SPAWN[2]);
    assert!(
        !column.flooded(),
        "the demo must open on dry ground: {column:?}"
    );
    assert_eq!(
        column.height, SPAWN[1],
        "the named spawn height must be the generated ground"
    );
    assert!(
        landmarks::covering(SEED, SPAWN[0], SPAWN[2]).is_none(),
        "the spawn must not be inside a landmark footprint"
    );
    // Ground a player can walk off and not a pit to stand in: at least one of
    // the eight neighbours is within a one-voxel step, and none of them is a
    // drop the spawn would trap the player in. The walk itself is a separate
    // test: it searches the whole approach to the nearest landmark.
    let mut nearest_step = i32::MAX;
    let mut furthest_step = 0;
    for (dx, dz) in [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ] {
        let neighbour = landscape::height_at(SEED, SPAWN[0] + dx, SPAWN[2] + dz);
        nearest_step = nearest_step.min((neighbour - SPAWN[1]).abs());
        furthest_step = furthest_step.max((neighbour - SPAWN[1]).abs());
    }
    println!("spawn steps: nearest {nearest_step} m, furthest {furthest_step} m");
    assert!(
        nearest_step <= 1,
        "the spawn must have a one-voxel step out, nearest was {nearest_step} m"
    );
    assert!(
        furthest_step <= 4,
        "the spawn must not stand in a pit: furthest step {furthest_step} m"
    );
}

#[test]
fn three_biomes_are_a_walk_from_the_demo_spawn() {
    let mut area: BTreeMap<&'static str, i64> = BTreeMap::new();
    let mut nearest: BTreeMap<&'static str, i32> = BTreeMap::new();
    let mut surface: BTreeMap<&'static str, BTreeMap<u8, i64>> = BTreeMap::new();
    let mut pressure: BTreeMap<&'static str, [i64; 2]> = BTreeMap::new();
    let step = 4;
    let span = WALK_RADIUS_M / step;
    for dz in -span..=span {
        for dx in -span..=span {
            if dx * dx + dz * dz > span * span {
                continue;
            }
            let (x, z) = (SPAWN[0] + dx * step, SPAWN[2] + dz * step);
            let column = landscape::column(SEED, x, z);
            if column.flooded() {
                continue;
            }
            let name = column.biome.name();
            // 16 m^2 per sampled column at a 4 m lattice.
            *area.entry(name).or_default() += 16;
            let distance = ((dx * dx + dz * dz) as f64).sqrt() as i32 * step;
            let entry = nearest.entry(name).or_insert(i32::MAX);
            *entry = (*entry).min(distance);
            *surface
                .entry(name)
                .or_default()
                .entry(column.surface)
                .or_default() += 1;
            let pressures = pressure.entry(name).or_default();
            pressures[0] += i64::from(column.mix.pressure(matterweave_core::Biome::grass_density));
            pressures[1] += i64::from(column.mix.pressure(matterweave_core::Biome::tree_density));
        }
    }
    let mut present: Vec<(&'static str, i64)> = area
        .iter()
        .filter(|(_, size)| **size >= BIOME_AREA_M2)
        .map(|(name, size)| (*name, *size))
        .collect();
    present.sort_by_key(|(_, size)| -size);
    let minutes = WALK_RADIUS_M as f32 / WALK_SPEED_MPS / 60.0;
    println!(
        "within {WALK_RADIUS_M} m of the demo spawn ({minutes:.1} minutes at {WALK_SPEED_MPS} m/s):"
    );
    for (name, size) in &present {
        let modal = surface[*name]
            .iter()
            .max_by_key(|(material, count)| (**count, **material))
            .map(|(material, _)| *material)
            .unwrap();
        let samples = area[*name] / 16;
        let mean = pressure[*name];
        println!(
            "  {name:<9} {size:>9} m2  nearest {:>4} m  ground {:<8} grass {:>2} tree {:>2}",
            nearest[*name],
            material::name(modal),
            mean[0] / samples,
            mean[1] / samples,
        );
    }
    assert!(
        present.len() >= 3,
        "three biomes must be inside a few minutes of the spawn: {present:?}"
    );
    let (first, second, third) = (present[0].0, present[1].0, present[2].0);
    for name in [first, second, third] {
        assert!(
            nearest[name] <= 400,
            "biome {name} has its nearest ground {} m away, further than a short walk",
            nearest[name]
        );
    }
    // "Visually distinct" is measured, not asserted: a screenshot at eye level
    // tells them apart because the ground material differs, and because the
    // vegetation form or its density differs.
    for (left, right) in [(first, second), (second, third), (first, third)] {
        let modal = |name: &'static str| {
            *surface[name]
                .iter()
                .max_by_key(|(material, count)| (**count, **material))
                .map(|(material, _)| material)
                .unwrap()
        };
        let (a, b) = (modal(left), modal(right));
        assert_ne!(
            a,
            b,
            "{left} and {right} show the same ground material {}",
            material::name(a)
        );
        let (ca, cb) = (material::color(a), material::color(b));
        let distance =
            ((ca[0] - cb[0]).powi(2) + (ca[1] - cb[1]).powi(2) + (ca[2] - cb[2]).powi(2)).sqrt();
        assert!(
            distance >= 0.2,
            "{left} ({}) and {right} ({}) are too close in colour: {distance:.3}",
            material::name(a),
            material::name(b)
        );
        let samples = |name: &'static str| area[name] / 16;
        let (pa, pb) = (pressure[left], pressure[right]);
        let differs =
            (0..2).any(|slot| (pa[slot] / samples(left) - pb[slot] / samples(right)).abs() >= 3);
        assert!(
            differs,
            "{left} and {right} carry the same vegetation densities: {pa:?} {pb:?}"
        );
    }
}

#[test]
fn the_spawn_view_frames_landmarks_and_the_start_of_a_transition() {
    let eye = eye();
    let mut visible: Vec<(i32, Landmark)> = Vec::new();
    for landmark in landmarks_within(2100) {
        let range = landmark.range_from(SPAWN[0], SPAWN[2]);
        if !(300..=2000).contains(&range) {
            continue;
        }
        if angle_off_axis(landmark.centre).abs() > WEDGE_HALF_ANGLE_DEGREES {
            continue;
        }
        if !summit_is_visible(eye, summit(&landmark)) {
            continue;
        }
        visible.push((range, landmark));
    }
    visible.sort_by_key(|(range, _)| *range);
    let mut above: Vec<i32> = visible
        .iter()
        .map(|(_, landmark)| visible_height_m(eye, landmark))
        .collect();
    println!(
        "visible from the spawn camera (eye {eye:?}), wedge +-{WEDGE_HALF_ANGLE_DEGREES} deg, \
         pixels at {VIEWPORT_PIXELS:.0} tall and {VIEW_FOV_DEGREES:.0} deg:"
    );
    for ((range, landmark), above) in visible.iter().zip(&above) {
        println!(
            "  {:<9} {:>5} m  off axis {:>6.1} deg  tall {:>3} m  subtends {:>5.1} px  \
             visible {:>3} m = {:>5.1} px",
            landmark.name(),
            range,
            angle_off_axis(landmark.centre),
            landmark.height_m(),
            silhouette_pixels(landmark.height_m(), *range),
            above,
            silhouette_pixels(*above, *range),
        );
    }
    assert!(
        visible.len() >= 2,
        "at least two landmarks must frame the spawn view: {visible:?}"
    );
    above.sort_unstable_by(|a, b| b.cmp(a));
    assert!(
        above[0] >= 40 && above[1] >= 16,
        "two landmarks must stand hundreds of metres of ground above the skyline, \
         not just clear it: {above:?} m"
    );
    let mut kinds: Vec<LandmarkKind> = visible.iter().map(|(_, landmark)| landmark.kind).collect();
    kinds.sort_by_key(|kind| kind.name());
    kinds.dedup();
    assert!(
        kinds.len() >= 2,
        "the framed landmarks must be recognisably different: {kinds:?}"
    );
    let nearest = visible[0].0;
    assert!(
        silhouette_pixels(visible[0].1.height_m(), nearest) >= 20.0,
        "the nearest landmark must be more than a smudge: {:?}",
        visible[0]
    );

    // The transition must start inside the same wedge, close enough to read -
    // and it must be a transition a walker can see, between two biomes that lay
    // down different ground. A plains/hills band is the same moss on both sides.
    let mut start = None;
    for radius in 1..300 {
        let bearing = YAW + ((radius * 7) % 81) as f64 - 40.0;
        let (sin, cos) = bearing.to_radians().sin_cos();
        let x = SPAWN[0] + (sin * radius as f64) as i32;
        let z = SPAWN[2] + (cos * radius as f64) as i32;
        let mix = landscape::biome_mix(SEED, x, z);
        if !mix.mixed(48) {
            continue;
        }
        let materials: Vec<u8> = mix
            .entries()
            .iter()
            .map(|&(biome, _)| landscape::biome_surface(SEED, x, z, biome))
            .collect();
        if materials.iter().any(|material| *material != materials[0]) {
            start = Some((radius, x, z, mix));
            break;
        }
    }
    let start = start.expect("a visible biome transition must begin inside the spawn view wedge");
    println!(
        "transition begins {} m ahead at ({}, {}) with {:?}",
        start.0,
        start.1,
        start.2,
        start.3.entries()
    );
    assert!(
        start.0 <= 200,
        "the beginning of a transition must be inside the first 200 m: {} m",
        start.0
    );
}

#[test]
fn landmark_footprints_are_walkable_ground() {
    let mut checked = 0;
    for landmark in landmarks_within(3000) {
        let radius = landmark.radius_m();
        let base = landscape::base_height(SEED, landmark.centre[0], landmark.centre[1]);
        assert!(
            base >= landmark.kind.min_base_m() && base <= landmark.kind.max_base_m(),
            "{landmark:?} stands on ground it cannot carry its height from: {base}"
        );
        // The silhouette is exactly the height the kind promises: the summit for
        // a raised kind, the rim for a crater, never clipped by the range clamp.
        let summit = base + landmark.height_m();
        assert!(
            summit < MAX_SURFACE_Y,
            "{landmark:?} summit hits the ceiling"
        );
        if landmark.kind == LandmarkKind::Crater {
            let rim = radius * 19 / 32;
            for (dx, dz) in [(rim, 0), (-rim, 0), (0, rim), (0, -rim)] {
                let (x, z) = (landmark.centre[0] + dx, landmark.centre[1] + dz);
                assert_eq!(
                    landscape::height_at(SEED, x, z),
                    landscape::base_height(SEED, x, z) + landmark.height_m(),
                    "{landmark:?} rim is not the height it claims at ({x}, {z})"
                );
            }
            assert!(
                landscape::height_at(SEED, landmark.centre[0], landmark.centre[1]) > 0,
                "{landmark:?} bowl is a lake"
            );
        } else {
            assert_eq!(
                landscape::height_at(SEED, landmark.centre[0], landmark.centre[1]),
                summit,
                "{landmark:?} summit is not the height it claims"
            );
        }

        // Walk the apron: the ring of ground a player stands on to reach the
        // landmark. Three claims, each measured rather than assumed:
        //
        // - Every dry sample is solid ground, and most of the ring is dry.
        // - The landmark's own lift has died out at the rim: it meets the
        //   ground it stands in instead of ending in a shelf.
        // - The landmark never makes the footprint harder to walk than the
        //   ground already was. A footprint inherits the terrain's own slopes -
        //   a spire on a coastal cliff has a cliff for a footprint, because the
        //   generator made one - so the bound is the walk slope, the terrain's
        //   own step when that is already steeper, plus the apron allowance the
        //   shaping is allowed to add.
        let shelf_limit = landmark.height_m() / 4;
        let mut previous: Option<(i32, i32, i32, i32)> = None;
        let (mut dry, mut steps) = (0, 0);
        let (mut worst_combined, mut worst_added) = (0.0f64, 0.0f64);
        for step in 0..128 {
            let (sin, cos) = (step as f64 * std::f64::consts::TAU / 128.0).sin_cos();
            let x = landmark.centre[0] + (sin * radius as f64) as i32;
            let z = landmark.centre[1] + (cos * radius as f64) as i32;
            let shaped = landscape::height_at(SEED, x, z);
            let ground = landscape::base_height(SEED, x, z);
            steps += 1;
            if shaped <= 0 {
                previous = None;
                continue;
            }
            dry += 1;
            assert!(
                (landmarks::shape(SEED, x, z, ground) - ground).abs() <= shelf_limit,
                "{landmark:?} lifts its footprint rim by more than {shelf_limit} m at ({x}, {z})"
            );
            if let Some((px, pz, pshaped, pground)) = previous {
                let run = (((x - px) as i64).pow(2) + ((z - pz) as i64).pow(2)) as f64;
                let run = run.sqrt();
                let shaped_rise = (shaped - pshaped).abs() as f64;
                let base_rise = (ground - pground).abs() as f64;
                let added = shaped_rise - base_rise;
                worst_combined = worst_combined.max(shaped_rise / run);
                worst_added = worst_added.max(added / run);
                let terrain_allowance =
                    base_rise.max(run * landmarks::WALK_SLOPE_PERMILLE as f64 / 1000.0);
                assert!(
                    shaped_rise
                        <= terrain_allowance
                            + run * landmarks::APRON_SLOPE_PERMILLE as f64 / 1000.0,
                    "{landmark:?} makes its footprint steeper than the ground: \
                     {shaped_rise:.1} m shaped against {base_rise:.1} m of ground over {run:.1} m"
                );
            }
            previous = Some((x, z, shaped, ground));
        }
        println!(
            "  {:<9} r {radius:>3} summit {summit:>3} apron dry {dry}/{steps} \
             worst slope {worst_combined:.2} (lift adds {worst_added:.2})",
            landmark.name()
        );
        assert!(
            dry * 2 >= steps,
            "{landmark:?} has more water than ground in its footprint: {dry}/{steps}"
        );
        checked += 1;
    }
    println!("{checked} landmark footprints walked inside 3 km");
    assert!(checked >= 3, "the demo region must contain landmarks");
}

#[test]
fn the_walk_from_the_spawn_reaches_the_nearest_landmark() {
    let target = landmarks_within(2100)
        .into_iter()
        .min_by_key(|landmark| landmark.range_from(SPAWN[0], SPAWN[2]))
        .expect("the demo region has landmarks");
    println!(
        "walking to {target:?} at {} m",
        target.range_from(SPAWN[0], SPAWN[2])
    );

    // Breadth-first over an 8 m lattice, with the walk slope as the only rule.
    const STEP: i32 = 8;
    const SLOPE_PERMILLE: i64 = landmarks::WALK_SLOPE_PERMILLE as i64;
    let span = 300; // +-2.4 km, enough for the nearest landmark and a detour
    let grid = (2 * span + 1) as usize;
    let origin = [SPAWN[0] - span * STEP, SPAWN[2] - span * STEP];
    let index = |gx: i32, gz: i32| (gz * (2 * span + 1) + gx) as usize;
    let start = (span, span);
    let mut visited = vec![false; grid * grid];
    let mut previous = vec![u32::MAX; grid * grid];
    let mut queue = VecDeque::new();
    visited[index(start.0, start.1)] = true;
    queue.push_back(start);
    let radius = target.radius_m();
    let mut goal = None;
    while let Some((gx, gz)) = queue.pop_front() {
        let (x, z) = (origin[0] + gx * STEP, origin[1] + gz * STEP);
        if target.range_from(x, z) <= radius {
            goal = Some((gx, gz));
            break;
        }
        let height = landscape::height_at(SEED, x, z);
        for (dx, dz) in [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ] {
            let (nx, nz) = (gx + dx, gz + dz);
            if nx < 0 || nz < 0 || nx > 2 * span || nz > 2 * span {
                continue;
            }
            let slot = index(nx, nz);
            if visited[slot] {
                continue;
            }
            let (wx, wz) = (origin[0] + nx * STEP, origin[1] + nz * STEP);
            let next = landscape::height_at(SEED, wx, wz);
            let run = if dx != 0 && dz != 0 { 11 } else { STEP };
            if (next - height).abs() as i64 * 1000 > run as i64 * SLOPE_PERMILLE {
                continue;
            }
            visited[slot] = true;
            previous[slot] = index(gx, gz) as u32;
            queue.push_back((nx, nz));
        }
    }
    let goal = goal.expect("the nearest landmark must be reachable on foot");
    let mut steps = 0usize;
    let mut cursor = index(goal.0, goal.1);
    while previous[cursor] != u32::MAX {
        cursor = previous[cursor] as usize;
        steps += 1;
    }
    let walked = steps as i32 * STEP;
    let straight = target.range_from(SPAWN[0], SPAWN[2]);
    let from_rim = straight - target.radius_m();
    println!(
        "reached the footprint in {steps} steps, about {walked} m against {from_rim} m from the rim 
         of a {straight} m straight line ({:.1}x, {} minutes at {WALK_SPEED_MPS} m/s)",
        walked as f32 / from_rim.max(1) as f32,
        walked as f32 / WALK_SPEED_MPS / 60.0,
    );
    assert!(
        walked < from_rim * 3,
        "the walk must not be a detour around an impassable wall: {walked} m for {from_rim} m"
    );
}

#[test]
fn landmarks_are_deterministic_and_seed_dependent() {
    for cell in -20..20 {
        let site = landmarks::site(SEED, cell, -cell);
        assert_eq!(site, landmarks::site(SEED, cell, -cell));
        assert_eq!(
            landmarks::landmark_at_cell(SEED, cell, -cell),
            landmarks::landmark_at_cell(SEED, cell, -cell)
        );
        if let Some(landmark) = site {
            let ground = landscape::base_height(SEED, landmark.centre[0], landmark.centre[1]);
            assert_eq!(
                landmarks::shape(SEED, landmark.centre[0], landmark.centre[1], ground),
                landscape::height_at(SEED, landmark.centre[0], landmark.centre[1])
            );
        }
    }
    let other = landmarks::sites_around(SEED + 1, [SPAWN[0], SPAWN[2]], 2000);
    let here = landmarks_within(2000);
    assert_ne!(
        here.iter().map(|l| l.centre).collect::<Vec<_>>(),
        other.iter().map(|l| l.centre).collect::<Vec<_>>(),
        "a different seed must place different landmarks"
    );
}

#[test]
fn the_two_other_kinds_are_in_the_world_too() {
    // Every kind must occur inside a 6 km square, or the demo is three towers.
    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    for landmark in landmarks::sites_around(SEED, [SPAWN[0], SPAWN[2]], 6000) {
        *kinds.entry(landmark.name()).or_default() += 1;
    }
    println!("kinds inside 6 km: {kinds:?}");
    for kind in LandmarkKind::ALL {
        assert!(
            kinds.contains_key(kind.name()),
            "kind {} never appears: {kinds:?}",
            kind.name()
        );
    }
}

#[test]
fn a_site_that_fails_its_ground_test_shapes_nothing() {
    // Ocean and ceiling rejections are the same function the listing uses, so a
    // rejected site must leave the column exactly as the fields made it.
    let mut rejected = 0;
    for cell_z in -12..12 {
        for cell_x in -12..12 {
            let Some(landmark) = landmarks::site(SEED, cell_x, cell_z) else {
                continue;
            };
            if landmarks::site_valid(SEED, &landmark) {
                continue;
            }
            rejected += 1;
            let ground = landscape::base_height(SEED, landmark.centre[0], landmark.centre[1]);
            assert_eq!(
                landscape::height_at(SEED, landmark.centre[0], landmark.centre[1]),
                ground,
                "{landmark:?} was rejected but still shaped its centre"
            );
        }
    }
    println!("{rejected} rejected sites in the sampled square, all leaving the ground alone");
    assert!(rejected > 0, "the sample must contain a rejected site");
}
