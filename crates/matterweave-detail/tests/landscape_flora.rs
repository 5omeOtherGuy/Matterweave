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
        let expected_scale = if id.starts_with("grass_tuft") || id.starts_with("flower_") {
            LANDSCAPE_FINE_FLORA_SCALE_M
        } else {
            LANDSCAPE_LEAF_SCALE_M
        };
        assert_eq!(volume.scale().metres(), expected_scale, "{id}");
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
    // Grass: a clump of one-voxel blades, 8..=20 fine voxels tall, each with a
    // tip several cells long, leaning outward from its own pad cell. The tiers
    // differ in clump radius as well as height, which is the per-site clump
    // radius variation the placement bridge selects with `scale_eighths`.
    let mut tier_mass = [0usize; 3];
    let mut tier_top = [0i32; 3];
    let mut tier_radius = [0i32; 3];
    for (index, (id, clump)) in [
        ("grass_tuft_s", grass_tuft_s("tuft_s").unwrap()),
        ("grass_tuft_m", grass_tuft_m("tuft_m").unwrap()),
        ("grass_tuft_l", grass_tuft_l("tuft_l").unwrap()),
    ]
    .into_iter()
    .enumerate()
    {
        let tips = clump
            .iter_cells()
            .filter(|(c, m)| *m == material::GRASS_TIP && c[1] > 0)
            .count();
        assert!(tips > 0, "{id} has tip material");
        let (min, max) = clump.cell_bounds().unwrap();
        assert_eq!(min[1], 0, "{id} is rooted");
        assert!(
            (GRASS_MIN_HEIGHT_CELLS..=GRASS_MAX_HEIGHT_CELLS).contains(&max[1]),
            "{id} is {} fine voxels tall, outside {}..={}",
            max[1],
            GRASS_MIN_HEIGHT_CELLS,
            GRASS_MAX_HEIGHT_CELLS
        );
        // Every blade stays a thin vertical element. The clump roots on a
        // one-cell-thick pad (the base set at y = 0 and the blade first cells at
        // y = 1), which is ground contact, not a body; above it, no cell may
        // have more than two occupied orthogonal neighbours at its own level,
        // so no level grows a plate or a cube. An elbow cell has exactly two
        // (the cell below it in the column and the cell it leaned to).
        let crowded = clump.iter_cells().filter(|(c, _)| c[1] > 1).any(|(c, _)| {
            [[1, 0], [-1, 0], [0, 1], [0, -1]]
                .iter()
                .filter(|[dx, dz]| clump.get([c[0] + dx, c[1], c[2] + dz]) != material::AIR)
                .count()
                > 2
        });
        assert!(!crowded, "{id} grows a plate or cube above its pad");
        tier_mass[index] = clump.occupied_cells();
        tier_top[index] = max[1];
        tier_radius[index] = max[0].max(max[2]).max(-min[0]).max(-min[2]);
    }
    // S/M/L stay distinct in height, mass *and* clump radius, so
    // `prototype_for`'s size tier means something beyond a label.
    assert!(
        tier_top[0] < tier_top[1] && tier_top[1] < tier_top[2],
        "tier heights must ascend: {tier_top:?}"
    );
    assert!(
        tier_mass[0] < tier_mass[1] && tier_mass[1] < tier_mass[2],
        "tier mass must ascend: {tier_mass:?}"
    );
    assert!(
        tier_radius[0] < tier_radius[1] && tier_radius[1] < tier_radius[2],
        "tier clump radii must ascend: {tier_radius:?}"
    );
    // Flowers: a cluster of thin stems of 8..=16 fine voxels, each carrying a
    // bloom. All nine colour/size flowers are geometrically distinct.
    let mut flower_shapes = BTreeSet::new();
    for id in LANDSCAPE_FLORA_SPECIES
        .iter()
        .filter(|id| id.starts_with("flower_"))
    {
        let v = landscape_prototype(id).unwrap();
        let (_, max) = v.cell_bounds().unwrap();
        assert!(
            (GRASS_MIN_HEIGHT_CELLS..=20).contains(&max[1]),
            "{id} blooms at {}, outside the 8..=20 stem range",
            max[1]
        );
        assert!(
            flower_shapes.insert(v.snapshot().runs),
            "{id} duplicates a shape"
        );
        // Every stem is one cell wide where it is green: the count of green
        // cells equals the sum of the stem lengths, with no thick posts.
        let green = v
            .iter_cells()
            .filter(|(_, m)| *m == material::GRASS_BLADE || *m == material::GRASS_TIP)
            .count();
        assert!(
            green >= 2 * GRASS_MIN_HEIGHT_CELLS as usize,
            "{id} carries {green} stem cells"
        );
        let hearts = v
            .iter_cells()
            .filter(|(_, m)| *m == material::FLOWER_HEART)
            .count();
        assert!(hearts >= 2, "{id} has {hearts} blooms");
    }
    assert_eq!(flower_shapes.len(), 9, "nine distinct flower shapes");
    // The three colours stay distinct petal materials.
    for (id, petal) in [
        ("flower_red_m", material::FLOWER_PETAL_RED),
        ("flower_white_m", material::FLOWER_PETAL_WHITE),
        ("flower_yellow_m", material::FLOWER_PETAL_YELLOW),
    ] {
        let v = landscape_prototype(id).unwrap();
        assert!(
            v.iter_cells().any(|(_, mat)| mat == petal),
            "{id} carries its petal material"
        );
    }
    // Trees: the trunk is a real trunk and the crown is a structured mass, not
    // a solid blob. Three checks, all on the source volume:
    for id in [
        "tree_broadleaf_s",
        "tree_broadleaf_m",
        "tree_broadleaf_l",
        "tree_conifer_s",
        "tree_conifer_m",
        "tree_conifer_l",
    ] {
        let tree = landscape_prototype(id).unwrap();
        let (min, max) = tree.cell_bounds().unwrap();
        assert!(max[1] >= 19, "{id} is {} cells tall", max[1]);
        // 1. A trunk stands at the origin from the ground into the crown.
        let trunk_cells = tree
            .iter_cells()
            .filter(|(_, m)| *m == material::TREE_BARK)
            .count();
        assert!(trunk_cells >= 8, "{id} has no trunk ({trunk_cells} bark)");
        assert!(
            tree.get([0, 1, 0]) == material::TREE_BARK,
            "{id} is not rooted in a trunk"
        );
        // 2. The crown is hollow: its bounding box is mostly air. A solid
        // ellipsoid of the same extent would fill far more of it.
        let box_cells = ((max[0] - min[0] + 1) as f64)
            * ((max[1] - min[1] + 1) as f64)
            * ((max[2] - min[2] + 1) as f64);
        let fill = tree.occupied_cells() as f64 / box_cells;
        assert!(fill < 0.25, "{id} fills {fill:.3} of its box, not hollow");
        // 3. Light passes through the crown: over the front projection, at
        // least a quarter of the columns that meet the crown show more than
        // one occupied run - a real gap between fronds, not a hole-free mass.
        let crown_floor = (max[1] * 2) / 5;
        let mut columns = 0usize;
        let mut gapped = 0usize;
        for x in min[0]..=max[0] {
            let column: Vec<bool> = (crown_floor..=max[1])
                .map(|y| tree.get([x, y, 0]) != material::AIR)
                .collect();
            if !column.iter().any(|c| *c) {
                continue;
            }
            columns += 1;
            let runs = column
                .split(|occupied| !*occupied)
                .filter(|run| !run.is_empty())
                .count();
            if runs >= 2 {
                gapped += 1;
            }
        }
        assert!(columns > 0, "{id} crown projects no columns");
        assert!(
            gapped * 4 >= columns,
            "{id}: only {gapped} of {columns} crown columns show a gap"
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
    // Conifer rings shrink towards the top.
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
        ring_width(8) > ring_width(24),
        "rings shrink towards the top"
    );
    let (_, top) = conifer.cell_bounds().unwrap();
    assert_eq!(top[1], 29, "medium conifer is 28 + tip cells tall");
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
