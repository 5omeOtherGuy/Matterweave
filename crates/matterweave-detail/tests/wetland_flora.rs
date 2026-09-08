//! Wetland support-flora acceptance: three new non-fungal prototypes.
//!
//! Covers the worker DoD: actual shape construction (not placeholders), one
//! six-connected body rooted at y = 0, deterministic round-trip, explicit
//! material policy (wood collides, leaves/pads do not), anatomy (branches and
//! leaves are not cylinders), all three LODs meshing within budget with the
//! source preserved, and the bracket species staying reserved.

use matterweave_detail::*;
use std::collections::BTreeSet;

/// Six-connected component count over occupied cells.
fn components(volume: &DetailVolume) -> usize {
    let cells: BTreeSet<[i32; 3]> = volume.iter_cells().map(|(c, _)| c).collect();
    let mut seen: BTreeSet<[i32; 3]> = BTreeSet::new();
    let mut count = 0;
    for start in &cells {
        if seen.contains(start) {
            continue;
        }
        count += 1;
        let mut stack = vec![*start];
        seen.insert(*start);
        while let Some(c) = stack.pop() {
            for step in [
                [1, 0, 0],
                [-1, 0, 0],
                [0, 1, 0],
                [0, -1, 0],
                [0, 0, 1],
                [0, 0, -1],
            ] {
                let n = [c[0] + step[0], c[1] + step[1], c[2] + step[2]];
                if cells.contains(&n) && seen.insert(n) {
                    stack.push(n);
                }
            }
        }
    }
    count
}

type Builder = fn(&str) -> Result<DetailVolume>;

fn builders() -> Vec<(&'static str, Builder)> {
    vec![
        ("twisted_shrub", twisted_shrub),
        ("horsetail", horsetail),
        ("marsh_lily", marsh_lily),
    ]
}

#[test]
fn catalogue_lists_three_species_and_bracket_stays_reserved() {
    assert_eq!(WETLAND_FLORA_SPECIES.len(), 3);
    assert!(WETLAND_FLORA_SPECIES.contains(&"twisted_shrub"));
    assert!(WETLAND_FLORA_SPECIES.contains(&"horsetail"));
    assert!(WETLAND_FLORA_SPECIES.contains(&"marsh_lily"));
    assert!(!WETLAND_FLORA_SPECIES.contains(&"bracket_shelf"));
    assert_eq!(
        wetland_prototype("bracket_shelf").err(),
        Some(DetailError::UnknownPrototype("bracket_shelf".into()))
    );
    for id in WETLAND_FLORA_SPECIES {
        wetland_prototype(id).expect("catalogue id builds");
    }
}

#[test]
fn bodies_are_connected_rooted_and_within_budget() {
    for (id, build) in builders() {
        let volume = build(id).unwrap();
        let (min, max) = volume.cell_bounds().unwrap();
        assert_eq!(min[1], 0, "{id} is rooted at local y = 0");
        assert!(max[1] > min[1] + 2, "{id} has vertical extent");
        assert_eq!(components(&volume), 1, "{id} is one connected body");
        assert!(
            volume.occupied_cells() < 33_288,
            "{id} fits the mesh preflight cap: {}",
            volume.occupied_cells()
        );
        assert!(volume.chunk_count() <= MAX_VOLUME_CHUNKS, "{id}");
        assert!(volume.occupied_cells() > 200, "{id} is not a placeholder");
        // Compact ground-contact footprint, not a floating canopy.
        assert!(
            volume.iter_cells().filter(|(c, _)| c[1] == 0).count() >= 5,
            "{id} has a real root footprint"
        );
    }
    assert_eq!(twisted_shrub("s").unwrap().scale().metres(), 0.125);
    assert_eq!(horsetail("h").unwrap().scale().metres(), 0.0625);
    assert_eq!(marsh_lily("l").unwrap().scale().metres(), 0.125);
    assert_eq!(WETLAND_LEAF_SCALE_M, 0.125);
    assert_eq!(WETLAND_SPIRE_SCALE_M, 0.0625);
}

#[test]
fn prototypes_are_deterministic() {
    for (id, build) in builders() {
        let first = build(id).unwrap();
        let second = build("same-content").unwrap();
        assert_eq!(first.snapshot().runs, second.snapshot().runs, "{id}");
        assert_eq!(first.occupied_cells(), second.occupied_cells(), "{id}");
        assert_eq!(first.cell_bounds(), second.cell_bounds(), "{id}");
    }
}

#[test]
fn material_policy_is_explicit_wood_collides_leaves_do_not() {
    // No new palette IDs: every material used here is pre-existing.
    for (id, build) in builders() {
        let volume = build(id).unwrap();
        for (_, m) in volume.iter_cells() {
            assert_ne!(material_name(m), "unknown", "{id} uses material {m}");
        }
    }
    // Woody trunk/branches reuse the collidable stipe solid; every leaf, pad,
    // needle, petal and reed-like stem is decorative.
    let shrub = twisted_shrub("shrub").unwrap();
    let wood = shrub
        .iter_cells()
        .filter(|(_, m)| *m == material::FLORA_FUNNEL_STIPE)
        .count();
    assert!(wood > 100, "substantive woody trunk, got {wood}");
    assert_eq!(
        material_policy(material::FLORA_FUNNEL_STIPE),
        MaterialPolicy::Collision
    );
    for (_, m) in shrub
        .iter_cells()
        .filter(|(_, m)| *m != material::FLORA_FUNNEL_STIPE)
    {
        assert_eq!(
            material_policy(m),
            MaterialPolicy::Decorative,
            "shrub foliage material {m} is decorative"
        );
    }
    assert!(assert_policy(
        &horsetail("h").unwrap(),
        MaterialPolicy::Decorative
    ));
    assert!(assert_policy(
        &marsh_lily("l").unwrap(),
        MaterialPolicy::Decorative
    ));
    // No decorative holes inside the collidable wood.
    assert_eq!(
        shrub
            .iter_cells()
            .filter(|(_, m)| *m == material::FLORA_LUMEN_DOT)
            .count(),
        0,
        "no decorative accent inside collidable wood"
    );
}

#[test]
fn shrub_has_twist_taper_and_leaf_crowns_not_cylinders() {
    let shrub = twisted_shrub("shrub").unwrap();
    let wood_at = |y: i32| {
        let xs: Vec<i32> = shrub
            .iter_cells()
            .filter(|(c, m)| c[1] == y && *m == material::FLORA_FUNNEL_STIPE)
            .map(|(c, _)| c[0])
            .collect();
        xs
    };
    // Twist: trunk wood centre moves across heights (not a straight cylinder).
    let centre = |y: i32| {
        let xs = wood_at(y);
        assert!(!xs.is_empty(), "trunk wood at y = {y}");
        xs.iter().sum::<i32>() / xs.len() as i32
    };
    let centres: Vec<i32> = [2, 5, 8, 11].map(centre).to_vec();
    assert!(
        centres.iter().max().unwrap() - centres.iter().min().unwrap() >= 2,
        "twisted trunk centres: {centres:?}"
    );
    // Taper: foot cross-section is wider than the waist.
    assert!(wood_at(1).len() > wood_at(8).len(), "tapered trunk");
    // Crowns dominate: leaf cells far outnumber wood cells, and veins exist.
    let count = |m: u8| shrub.iter_cells().filter(|(_, mat)| *mat == m).count();
    let leaves = count(material::FLORA_FROND_BLADE) + count(material::FLORA_ROSETTE_LEAF);
    let wood = count(material::FLORA_FUNNEL_STIPE);
    assert!(leaves > wood, "crowns {leaves} dominate wood {wood}");
    assert!(count(material::FLORA_FROND_RIB) > 0, "crown veins");
}

#[test]
fn horsetail_has_nodes_whorls_and_cone_not_equal_culms() {
    let horse = horsetail("horse").unwrap();
    let count = |m: u8| horse.iter_cells().filter(|(_, mat)| *mat == m).count();
    // Joint bands wider than internodes: rib ring cells at a node exceed the
    // stem cross-section just below it.
    let node_rib = horse
        .iter_cells()
        .filter(|(c, m)| c[1] == 9 && *m == material::FLORA_FROND_RIB)
        .count();
    let inter_stem = horse
        .iter_cells()
        .filter(|(c, m)| c[1] == 8 && *m == material::FLORA_REED_STEM)
        .count();
    assert!(
        node_rib > inter_stem,
        "node {node_rib} wider than internode {inter_stem}"
    );
    assert!(count(material::FLORA_REED_LEAF) >= 100, "whorled needles");
    assert!(count(material::FLORA_REED_PLUME) >= 20, "spore cone");
    // One dominant spire plus two side shoots: the plume cells form exactly
    // three connected cone masses (one per stem tip).
    let plume: BTreeSet<[i32; 3]> = horse
        .iter_cells()
        .filter(|(_, m)| *m == material::FLORA_REED_PLUME)
        .map(|(c, _)| c)
        .collect();
    let mut seen: BTreeSet<[i32; 3]> = BTreeSet::new();
    let mut cones = 0;
    for start in &plume {
        if seen.contains(start) {
            continue;
        }
        cones += 1;
        let mut stack = vec![*start];
        seen.insert(*start);
        while let Some(c) = stack.pop() {
            for step in [
                [1, 0, 0],
                [-1, 0, 0],
                [0, 1, 0],
                [0, -1, 0],
                [0, 0, 1],
                [0, 0, -1],
            ] {
                let n = [c[0] + step[0], c[1] + step[1], c[2] + step[2]];
                if plume.contains(&n) && seen.insert(n) {
                    stack.push(n);
                }
            }
        }
    }
    assert_eq!(cones, 3, "spire plus two side-shoot cones");
    // Tallest of the three species: vertical accent.
    let (_, max) = horse.cell_bounds().unwrap();
    assert!(max[1] >= 30, "spire height in 6.25 cm cells");
}

#[test]
fn lily_has_flat_notched_pads_and_a_flower() {
    let lily = marsh_lily("lily").unwrap();
    let (min, max) = lily.cell_bounds().unwrap();
    let width = (max[0] - min[0]).max(max[2] - min[2]);
    let height = max[1] - min[1];
    assert!(
        width >= 3 * height,
        "broad flat colony: {width} vs {height}"
    );
    // The leftmost pad is centred at (-7,2); its radial slit opens +X.
    // Check the slit itself and its Z-flanking walls, not gaps between pads.
    for x in -6..=-4 {
        assert_eq!(lily.get([x, 2, 2]), material::AIR, "notch slit");
        assert_ne!(lily.get([x, 2, 1]), material::AIR, "north notch wall");
        assert_ne!(lily.get([x, 2, 3]), material::AIR, "south notch wall");
    }
    let count = |m: u8| lily.iter_cells().filter(|(_, mat)| *mat == m).count();
    assert!(count(material::FLORA_ROSETTE_LEAF) > 50, "pad flesh");
    assert!(count(material::FLORA_ROSETTE_SPOT) >= 5, "flower petals");
    assert_eq!(count(material::FLORA_LUMEN_DOT), 1, "one flower heart");
    // Rhizome runner at y = 0 is the only ground contact.
    assert!(
        lily.iter_cells()
            .any(|(c, m)| c[1] == 0 && m == material::FLORA_REED_STEM),
        "rhizome feet"
    );
}

#[test]
fn wetland_prototypes_round_trip_and_survive_lod_derivation() {
    for (id, build) in builders() {
        let volume = build(id).unwrap();
        let snapshot = volume.snapshot();
        let restored = DetailVolume::from_snapshot(&snapshot).unwrap();
        assert_eq!(restored.occupied_cells(), volume.occupied_cells(), "{id}");
        assert_eq!(restored.cell_bounds(), volume.cell_bounds(), "{id}");
        assert_eq!(restored.snapshot().runs, snapshot.runs, "{id}");

        let before = volume.snapshot();
        let half = volume.coarsen(Lod::Half).unwrap();
        let quarter = volume.coarsen(Lod::Quarter).unwrap();
        assert_eq!(volume.snapshot(), before, "{id} source intact across LOD");
        assert!(
            half.occupied_cells() > 0 && quarter.occupied_cells() > 0,
            "{id}"
        );
        assert!(half.occupied_cells() <= volume.occupied_cells(), "{id}");

        let mut scene = DetailScene::new();
        scene.add_prototype(volume.clone()).unwrap();
        for lod in [Lod::Source, Lod::Half, Lod::Quarter] {
            let standalone = match lod {
                Lod::Source => volume.mesh_local().unwrap(),
                other => volume.coarsen(other).unwrap().mesh_local().unwrap(),
            };
            assert!(
                !standalone.indices.is_empty(),
                "{id} {lod:?} meshes geometry"
            );
            let scene_mesh = scene.prototype_mesh(id, lod).unwrap();
            assert_eq!(
                scene_mesh.indices.len(),
                standalone.indices.len(),
                "{id} {lod:?}"
            );
            assert_eq!(
                scene_mesh.revision,
                volume.revision(),
                "{id} {lod:?} carries the authoritative source revision"
            );
        }
        assert_eq!(
            volume.snapshot(),
            before,
            "{id} source intact after meshing"
        );
    }
}
