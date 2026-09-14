//! Landscape flora catalogue acceptance: materials, prototypes, budgets,
//! placement bridge and per-square budget.
//!
//! Covers the worker DoD: additive Decorative materials 60..=72 with existing
//! IDs untouched, 21 voxel-built prototypes that are each one connected body
//! rooted at y = 0, enforced cell and meshed-triangle caps, deterministic
//! builds and meshes, LOD coarsening for every prototype, a total
//! [`prototype_for`] bridge over [`FloraKind`], and biome-driven placement
//! (plains grass/flowers, forest trees, desert cacti) inside the documented
//! per-16-m-square budget.

use matterweave_core::landscape::{
    biome_at, flora_cell, tree_cell, Biome, FloraKind, FloraSite, FLORA_CELL_M, TREE_CELL_M,
};
use matterweave_detail::*;
use std::collections::BTreeSet;

/// Six-connected component count over occupied cells (worker-local copy; the
/// module itself exposes [`is_single_body`], this recounts from scratch).
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

fn cell_cap(id: &str) -> usize {
    if id.starts_with("grass_tuft") {
        GRASS_CELL_CAP
    } else if id.starts_with("flower_") {
        FLOWER_CELL_CAP
    } else if id == "fern" {
        FERN_CELL_CAP
    } else if id == "shrub" {
        SHRUB_CELL_CAP
    } else if id == "cactus" {
        CACTUS_CELL_CAP
    } else {
        TREE_CELL_CAP
    }
}

fn tri_cap(id: &str) -> usize {
    if id.starts_with("grass_tuft") {
        GRASS_TRI_CAP
    } else if id.starts_with("flower_") {
        FLOWER_TRI_CAP
    } else if id == "fern" {
        FERN_TRI_CAP
    } else if id == "shrub" {
        SHRUB_TRI_CAP
    } else if id == "cactus" {
        CACTUS_TRI_CAP
    } else {
        TREE_TRI_CAP
    }
}

fn mesh_tris(id: &str) -> usize {
    let volume = landscape_prototype(id).unwrap();
    let mut scene = DetailScene::new();
    scene.add_prototype(volume).unwrap();
    scene.prototype_mesh(id, Lod::Source).unwrap().indices.len() / 3
}

#[test]
fn catalogue_lists_every_required_id_and_builds() {
    // 3 grass + 9 flowers + fern + shrub + cactus + 3 broadleaf + 3 conifer.
    assert_eq!(LANDSCAPE_FLORA_SPECIES.len(), 21);
    for id in LANDSCAPE_FLORA_SPECIES {
        landscape_prototype(id).expect("catalogue id builds");
    }
    assert_eq!(
        landscape_prototype("bracket_shelf").err(),
        Some(DetailError::UnknownPrototype("bracket_shelf".into()))
    );
    // Reuse ids resolve through the same entry point without duplication.
    assert!(landscape_prototype("reed_cluster").is_ok());
    assert!(landscape_prototype("rosette_groundcover").is_ok());
}

#[test]
fn new_materials_are_decorative_and_existing_ids_unchanged() {
    for id in 60..=72u8 {
        assert_eq!(
            material_policy(id),
            MaterialPolicy::Decorative,
            "material {id}"
        );
        assert_ne!(material_name(id), "unknown", "material {id}");
        assert!(material_color(id).iter().all(|v| v.is_finite()));
    }
    assert_eq!(material_name(material::GRASS_BLADE), "grass_blade");
    assert_eq!(material_name(material::GRASS_TIP), "grass_tip");
    assert_eq!(
        material_name(material::FLOWER_PETAL_RED),
        "flower_petal_red"
    );
    assert_eq!(
        material_name(material::FLOWER_PETAL_WHITE),
        "flower_petal_white"
    );
    assert_eq!(
        material_name(material::FLOWER_PETAL_YELLOW),
        "flower_petal_yellow"
    );
    assert_eq!(material_name(material::FLOWER_HEART), "flower_heart");
    assert_eq!(material_name(material::SHRUB_LEAF), "shrub_leaf");
    assert_eq!(material_name(material::SHRUB_STEM), "shrub_stem");
    assert_eq!(material_name(material::TREE_BARK), "tree_bark");
    assert_eq!(material_name(material::TREE_LEAF), "tree_leaf");
    assert_eq!(material_name(material::TREE_NEEDLE), "tree_needle");
    assert_eq!(material_name(material::CACTUS_BODY), "cactus_body");
    assert_eq!(material_name(material::CACTUS_SPINE), "cactus_spine");
    // Existing meanings are preserved exactly.
    assert_eq!(material_name(material::MUSHROOM_CAP), "mushroom_cap");
    assert_eq!(
        material_name(material::FLORA_FUNGUS_LUMEN),
        "flora_fungus_lumen"
    );
    assert_eq!(material_policy(material::WATER), MaterialPolicy::Liquid);
    assert_eq!(
        material_policy(material::FLORA_FUNNEL_CAP),
        MaterialPolicy::Collision
    );
    assert_eq!(
        material_policy(material::FLORA_REED_LEAF),
        MaterialPolicy::Decorative
    );
}

#[test]
fn prototypes_are_connected_rooted_decorative_and_within_cell_caps() {
    for id in LANDSCAPE_FLORA_SPECIES {
        let volume = landscape_prototype(id).unwrap();
        let (min, max) = volume.cell_bounds().unwrap();
        assert_eq!(min[1], 0, "{id} is rooted at local y = 0");
        assert!(max[1] > min[1], "{id} has vertical extent");
        assert_eq!(components(&volume), 1, "{id} is one connected body");
        assert!(is_single_body(&volume), "{id}");
        assert!(
            volume.iter_cells().any(|(c, _)| c[1] == 0),
            "{id} has ground contact"
        );
        assert!(
            volume.occupied_cells() <= cell_cap(id),
            "{id} cells {} over cap {}",
            volume.occupied_cells(),
            cell_cap(id)
        );
        assert!(volume.chunk_count() <= MAX_VOLUME_CHUNKS, "{id}");
        assert_eq!(volume.scale().metres(), LANDSCAPE_LEAF_SCALE_M, "{id}");
        assert!(assert_landscape_policy(&volume), "{id} is fully Decorative");
        assert!(assert_policy(&volume, MaterialPolicy::Decorative), "{id}");
    }
}

#[test]
fn prototypes_are_deterministic_with_stable_mesh_bytes() {
    for id in LANDSCAPE_FLORA_SPECIES {
        let first = landscape_prototype(id).unwrap();
        let second = landscape_prototype(id).unwrap();
        assert_eq!(first.snapshot().runs, second.snapshot().runs, "{id}");
        assert_eq!(first.occupied_cells(), second.occupied_cells(), "{id}");
        assert_eq!(first.cell_bounds(), second.cell_bounds(), "{id}");
        // Meshing twice gives equal bytes: compare serialized f32/u32 output.
        let a = first.mesh_local().unwrap();
        let b = first.mesh_local().unwrap();
        assert_eq!(a.vertices.len(), b.vertices.len(), "{id}");
        assert_eq!(a.indices.len(), b.indices.len(), "{id}");
        let flat = |m: &matterweave_core::Mesh| {
            let mut bytes = Vec::new();
            for v in &m.vertices {
                for x in v.position.iter().chain(&v.normal).chain(&v.color) {
                    bytes.extend_from_slice(&x.to_le_bytes());
                }
            }
            for i in &m.indices {
                bytes.extend_from_slice(&i.to_le_bytes());
            }
            bytes
        };
        assert_eq!(flat(&a), flat(&b), "{id} mesh bytes");
    }
}

#[test]
fn meshed_triangle_caps_hold_through_the_scene_path() {
    for id in LANDSCAPE_FLORA_SPECIES {
        let tris = mesh_tris(id);
        assert!(
            tris <= tri_cap(id),
            "{id} triangles {tris} over cap {}",
            tri_cap(id)
        );
        assert!(tris > 0, "{id} meshes real geometry");
    }
}

#[test]
fn every_prototype_coarsens_with_strictly_fewer_cells() {
    for id in LANDSCAPE_FLORA_SPECIES {
        let volume = landscape_prototype(id).unwrap();
        let before = volume.snapshot();
        let half = volume.coarsen(Lod::Half).unwrap();
        let quarter = volume.coarsen(Lod::Quarter).unwrap();
        assert_eq!(volume.snapshot(), before, "{id} source intact across LOD");
        assert!(half.occupied_cells() > 0, "{id} half non-empty");
        assert!(quarter.occupied_cells() > 0, "{id} quarter non-empty");
        assert!(
            half.occupied_cells() < volume.occupied_cells(),
            "{id} half strictly coarsens"
        );
        assert!(
            quarter.occupied_cells() < half.occupied_cells(),
            "{id} quarter strictly coarsens"
        );
        assert!(
            !half.mesh_local().unwrap().indices.is_empty(),
            "{id} half meshes"
        );
        assert!(
            !quarter.mesh_local().unwrap().indices.is_empty(),
            "{id} quarter meshes"
        );
    }
}

fn site(kind: FloraKind, scale_eighths: u8) -> FloraSite {
    FloraSite {
        kind,
        x: 0,
        y: 0,
        z: 0,
        yaw_quarters: 0,
        scale_eighths,
    }
}

#[test]
fn prototype_for_is_total_and_uses_scale_variants() {
    let kinds = [
        FloraKind::GrassTuft,
        FloraKind::FlowerRed,
        FloraKind::FlowerWhite,
        FloraKind::FlowerYellow,
        FloraKind::Fern,
        FloraKind::Shrub,
        FloraKind::TreeBroadleaf,
        FloraKind::TreeConifer,
        FloraKind::Reed,
        FloraKind::Cactus,
    ];
    assert_eq!(kinds.len(), 10);
    for kind in kinds {
        // Every scale in the legal 4..=12 range maps to a prototype that builds.
        for scale in 4..=12u8 {
            let id = prototype_for(&site(kind, scale));
            landscape_prototype(id).unwrap_or_else(|_| panic!("{kind:?} -> {id} builds"));
        }
    }
    // Scale boundaries: <6 small, <10 medium, else large.
    assert_eq!(
        prototype_for(&site(FloraKind::GrassTuft, 4)),
        "grass_tuft_s"
    );
    assert_eq!(
        prototype_for(&site(FloraKind::GrassTuft, 5)),
        "grass_tuft_s"
    );
    assert_eq!(
        prototype_for(&site(FloraKind::GrassTuft, 6)),
        "grass_tuft_m"
    );
    assert_eq!(
        prototype_for(&site(FloraKind::GrassTuft, 9)),
        "grass_tuft_m"
    );
    assert_eq!(
        prototype_for(&site(FloraKind::GrassTuft, 10)),
        "grass_tuft_l"
    );
    assert_eq!(
        prototype_for(&site(FloraKind::GrassTuft, 12)),
        "grass_tuft_l"
    );
    assert_eq!(
        prototype_for(&site(FloraKind::TreeBroadleaf, 4)),
        "tree_broadleaf_s"
    );
    assert_eq!(
        prototype_for(&site(FloraKind::TreeConifer, 12)),
        "tree_conifer_l"
    );
    // Unsized kinds ignore scale; Reed reuses the existing reed prototype.
    assert_eq!(prototype_for(&site(FloraKind::Fern, 4)), "fern");
    assert_eq!(prototype_for(&site(FloraKind::Fern, 12)), "fern");
    assert_eq!(prototype_for(&site(FloraKind::Shrub, 8)), "shrub");
    assert_eq!(prototype_for(&site(FloraKind::Reed, 8)), "reed_cluster");
    assert_eq!(prototype_for(&site(FloraKind::Cactus, 8)), "cactus");
    // Flower colours stay distinct per kind at every scale.
    for scale in [4, 8, 12] {
        assert_eq!(
            prototype_for(&site(FloraKind::FlowerRed, scale)),
            format!("flower_red_{}", variant_suffix(scale))
        );
        assert_eq!(
            prototype_for(&site(FloraKind::FlowerWhite, scale)),
            format!("flower_white_{}", variant_suffix(scale))
        );
        assert_eq!(
            prototype_for(&site(FloraKind::FlowerYellow, scale)),
            format!("flower_yellow_{}", variant_suffix(scale))
        );
    }
}

fn variant_suffix(scale_eighths: u8) -> &'static str {
    if scale_eighths < 6 {
        "s"
    } else if scale_eighths < 10 {
        "m"
    } else {
        "l"
    }
}

#[test]
fn landscape_flora_classes_cover_every_species() {
    for id in LANDSCAPE_FLORA_SPECIES {
        let class = landscape_flora_class(id);
        assert!(
            [
                "flora-grass",
                "flora-flower",
                "flora-shrub",
                "flora-tree",
                "flora-cactus"
            ]
            .contains(&class),
            "{id} -> {class}"
        );
    }
    assert_eq!(landscape_flora_class("grass_tuft_m"), "flora-grass");
    assert_eq!(landscape_flora_class("flower_red_m"), "flora-flower");
    assert_eq!(landscape_flora_class("fern"), "flora-grass");
    assert_eq!(landscape_flora_class("shrub"), "flora-shrub");
    assert_eq!(landscape_flora_class("cactus"), "flora-cactus");
    assert_eq!(landscape_flora_class("tree_broadleaf_m"), "flora-tree");
    assert_eq!(landscape_flora_class("tree_conifer_l"), "flora-tree");
    // Reuse ids classify with the ground layer, never unknown.
    assert_eq!(landscape_flora_class("reed_cluster"), "flora-grass");
    assert_eq!(landscape_flora_class("rosette_groundcover"), "flora-flower");
}

#[test]
fn grass_and_tree_shapes_match_their_spec() {
    // Grass: a fan of GRASS_BLADES_PER_TUFT leaning blades, each with its top
    // GRASS_TIP_CELLS_PER_BLADE cells in the lighter tip material.
    let mut tier_mass = [0usize; 3];
    let mut tier_top = [0i32; 3];
    for (index, (id, tuft)) in [
        ("grass_tuft_s", grass_tuft_s("tuft_s").unwrap()),
        ("grass_tuft_m", grass_tuft_m("tuft_m").unwrap()),
        ("grass_tuft_l", grass_tuft_l("tuft_l").unwrap()),
    ]
    .into_iter()
    .enumerate()
    {
        let tips = tuft
            .iter_cells()
            .filter(|(_, m)| *m == material::GRASS_TIP)
            .count();
        assert_eq!(
            tips,
            GRASS_BLADES_PER_TUFT * GRASS_TIP_CELLS_PER_BLADE,
            "{id}: every blade carries its own tip cells"
        );
        // Every blade leaves the shared stem, so the tuft occupies more
        // columns above the pad than the stem alone, and the eight compass
        // leans put a blade in each of the eight blade columns.
        let (min, max) = tuft.cell_bounds().unwrap();
        assert_eq!((min[0], min[2]), (-1, -1), "{id} spans the 3x3 fan");
        assert_eq!((max[0], max[2]), (1, 1), "{id} spans the 3x3 fan");
        let blade_columns: BTreeSet<[i32; 2]> = tuft
            .iter_cells()
            .filter(|(c, m)| c[1] > 0 && *m == material::GRASS_BLADE)
            .map(|(c, _)| [c[0], c[2]])
            .collect();
        assert_eq!(
            blade_columns.len(),
            GRASS_BLADES_PER_TUFT + 1,
            "{id}: the eight blade columns plus the shared stem"
        );
        tier_mass[index] = tuft.occupied_cells();
        tier_top[index] = max[1];
    }
    // S/M/L stay distinct in height and mass, so `prototype_for`'s size tier
    // still means something beyond a label.
    assert!(
        tier_top[0] < tier_top[1] && tier_top[1] < tier_top[2],
        "tier heights must ascend: {tier_top:?}"
    );
    assert!(
        tier_mass[0] < tier_mass[1] && tier_mass[1] < tier_mass[2],
        "tier mass must ascend: {tier_mass:?}"
    );
    // Flowers: stem + two leaves + petal cross + heart, heights differ by colour.
    let red = flower_red_m("red").unwrap();
    let white = flower_white_m("white").unwrap();
    let yellow = flower_yellow_m("yellow").unwrap();
    let stem_len = |v: &DetailVolume| {
        (1..)
            .take_while(|y| v.get([0, *y, 0]) == material::GRASS_BLADE)
            .count()
    };
    let (r, w, y) = (stem_len(&red), stem_len(&white), stem_len(&yellow));
    assert!((2..=4).contains(&r) && (2..=4).contains(&w) && (2..=4).contains(&y));
    assert!(
        r != w && w != y && r != y,
        "meadow mixes silhouettes: {r}/{w}/{y}"
    );
    // Every flower stem stays in the 2..4 spec range, and all nine
    // colour/size flowers are geometrically distinct.
    let mut flower_shapes = BTreeSet::new();
    for id in LANDSCAPE_FLORA_SPECIES
        .iter()
        .filter(|id| id.starts_with("flower_"))
    {
        let v = landscape_prototype(id).unwrap();
        let stem = (1..)
            .take_while(|y| v.get([0, *y, 0]) == material::GRASS_BLADE)
            .count();
        assert!((2..=4).contains(&stem), "{id} stem {stem}");
        assert!(
            flower_shapes.insert(v.snapshot().runs),
            "{id} duplicates a shape"
        );
    }
    assert_eq!(flower_shapes.len(), 9, "nine distinct flower shapes");
    for (v, m) in [
        (&red, material::FLOWER_PETAL_RED),
        (&white, material::FLOWER_PETAL_WHITE),
        (&yellow, material::FLOWER_PETAL_YELLOW),
    ] {
        let petals = v.iter_cells().filter(|(_, mat)| *mat == m).count();
        assert!((3..=5).contains(&petals), "petal cross has 3..5 cells");
        assert_eq!(
            v.iter_cells()
                .filter(|(_, mat)| *mat == material::FLOWER_HEART)
                .count(),
            1,
            "one heart cell"
        );
        assert_eq!(
            v.iter_cells()
                .filter(|(c, mat)| c[1] > 0 && *mat == material::GRASS_BLADE)
                .count(),
            stem_len(v) + 2,
            "stem plus two leaf cells"
        );
    }
    // Cactus: column 5..9 tall with 2 arms and silhouette spines.
    let cactus = cactus("cactus").unwrap();
    let (_, max) = cactus.cell_bounds().unwrap();
    assert!((5..=9).contains(&max[1]), "column height {}", max[1]);
    assert!(
        cactus
            .iter_cells()
            .filter(|(_, m)| *m == material::CACTUS_SPINE)
            .count()
            >= 4,
        "spine cells on the silhouette"
    );
    // Conifer rings shrink towards the top; broadleaf canopy is rounded.
    let conifer = tree_conifer_m("conifer").unwrap();
    let ring_width = |y: i32| {
        let xs: Vec<i32> = conifer
            .iter_cells()
            .filter(|(c, m)| c[1] == y && *m == material::TREE_NEEDLE)
            .map(|(c, _)| c[0])
            .collect();
        xs.iter().max().unwrap_or(&0) - xs.iter().min().unwrap_or(&0)
    };
    assert!(
        ring_width(7) > ring_width(16),
        "rings shrink towards the top"
    );
    let (_, top) = conifer.cell_bounds().unwrap();
    assert_eq!(top[1], 19, "medium conifer is 18 + tip cells tall");
}

/// Collect every site the landscape population places in the 16 m square with
/// metre origin `(ox, oz)`: 8x8 flora cells plus 2x2 tree cells.
fn square_sites(seed: u64, ox: i32, oz: i32) -> Vec<FloraSite> {
    let mut sites = Vec::new();
    let (cx0, cz0) = (ox.div_euclid(FLORA_CELL_M), oz.div_euclid(FLORA_CELL_M));
    for cx in cx0..cx0 + 8 {
        for cz in cz0..cz0 + 8 {
            sites.extend(flora_cell(seed, cx, cz).sites().iter().copied());
        }
    }
    let (tx0, tz0) = (ox.div_euclid(TREE_CELL_M), oz.div_euclid(TREE_CELL_M));
    for tx in tx0..tx0 + 2 {
        for tz in tz0..tz0 + 2 {
            sites.extend(tree_cell(seed, tx, tz));
        }
    }
    sites
}

/// First 16 m square (scanning metre origins on a 16 m lattice) whose centre
/// column has `biome`, searching a bounded deterministic window.
fn find_square(seed: u64, biome: Biome) -> Option<(i32, i32)> {
    for gx in -64..64 {
        for gz in -64..64 {
            let (ox, oz) = (gx * 16, gz * 16);
            if biome_at(seed, ox + 8, oz + 8) == biome {
                return Some((ox, oz));
            }
        }
    }
    None
}

#[test]
fn biome_driven_placement_with_documented_square_budget() {
    let seed = 7u64;
    // Plains: grass tufts and flowers.
    let (ox, oz) = find_square(seed, Biome::Plains).expect("a plains square");
    let mut plains = square_sites(seed, ox, oz);
    // A single square can be unlucky; scan neighbours deterministically.
    let mut tries = 0;
    while tries < 8
        && !plains.iter().any(|s| s.kind == FloraKind::GrassTuft)
        && !plains.iter().any(|s| {
            matches!(
                s.kind,
                FloraKind::FlowerRed | FloraKind::FlowerWhite | FloraKind::FlowerYellow
            )
        })
    {
        tries += 1;
        plains = square_sites(seed, ox + tries * 16, oz);
    }
    assert!(
        plains.iter().any(|s| s.kind == FloraKind::GrassTuft),
        "plains places grass"
    );
    assert!(
        plains.iter().any(|s| matches!(
            s.kind,
            FloraKind::FlowerRed | FloraKind::FlowerWhite | FloraKind::FlowerYellow
        )),
        "plains places flowers"
    );
    // Forest: broadleaf or conifer trees via tree_cell.
    let (fx, fz) = find_square(seed, Biome::Forest).expect("a forest square");
    let mut forest = square_sites(seed, fx, fz);
    tries = 0;
    while tries < 8
        && !forest
            .iter()
            .any(|s| matches!(s.kind, FloraKind::TreeBroadleaf | FloraKind::TreeConifer))
    {
        tries += 1;
        forest = square_sites(seed, fx + tries * 16, fz);
    }
    assert!(
        forest
            .iter()
            .any(|s| { matches!(s.kind, FloraKind::TreeBroadleaf | FloraKind::TreeConifer) }),
        "forest places trees"
    );
    // Desert: cacti are rare per column, so scan boundedly for a square that has one.
    let (dx, dz) = find_square(seed, Biome::Desert).expect("a desert square");
    let mut desert = Vec::new();
    let mut found = false;
    for step in 0..40 {
        desert = square_sites(seed, dx + step * 16, dz);
        if desert.iter().any(|s| s.kind == FloraKind::Cactus) {
            found = true;
            break;
        }
    }
    assert!(found, "desert places cacti within a bounded scan");
    // Every bridge id builds, and every square stays inside the budget.
    for square in [&plains, &forest, &desert] {
        assert!(
            square.len() <= MAX_SITES_PER_16M_SQUARE,
            "square holds {} sites over budget {MAX_SITES_PER_16M_SQUARE}",
            square.len()
        );
        for placed_id in square.iter().map(prototype_for) {
            landscape_prototype(placed_id).expect("placed id builds");
        }
    }
}
