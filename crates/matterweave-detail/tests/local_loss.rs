//! Local thin-feature/opening safety: a bounded source-derived guard beyond the
//! global dilation heuristic.
//!
//! The global dilation fraction is negligible for a small tunnel bored through
//! a thick solid, so automatic selection used to coarsen it away. These
//! fixtures pin the new behavior: interior heterogeneous coarse cells hold the
//! prototype at `Source`, while dense solids and adjacent instances still
//! coarsen, under both perspective and orthographic projections.

use matterweave_detail::{
    material, Camera, DetailScene, DetailVolume, Lod, LodConfig, Projection, Scale, Transform, Yaw,
    SCALE_FINE_M,
};

const FOV60: f32 = std::f32::consts::PI / 3.0;
const STONE: u8 = material::BANK_STONE;

fn solid_block(id: &str, edge: i32) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).unwrap());
    for x in 0..edge {
        for y in 0..edge {
            for z in 0..edge {
                v.set([x, y, z], STONE).unwrap();
            }
        }
    }
    v
}

/// Thick 16-cell solid with a 1x1 through-tunnel along x at (y, z) = (8, 8).
/// Removed volume is 16 / 4096 cells; the global dilation stays ~0.004, far
/// below the default bias, so only the local interior-loss guard can hold it.
fn block_with_through_tunnel(id: &str) -> DetailVolume {
    let mut v = solid_block(id, 16);
    for x in 0..16 {
        v.set([x, 8, 8], material::AIR).unwrap();
    }
    v
}

/// Isolated 1x1x24 vertical stem: no coarse footprint fits inside its bounds,
/// so the local guard is silent and the global dilation bias holds it instead.
fn thin_stem(id: &str) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).unwrap());
    for z in 0..24 {
        v.set([0, 0, z], STONE).unwrap();
    }
    v
}

/// 13-cell solid (odd edge, so factor-2 boundary coarse cells exist) with a
/// 2-cell pit carved in the +x face at x == 12. The pit sits inside a coarse
/// cell whose footprint extends past the occupied bounds, which the
/// footprint-fully-inside guard ignores; global dilation is ~0.20 at Half,
/// well under the bias, so the pit is filled away by an unsafe Half choice.
fn block_with_face_pit(id: &str) -> DetailVolume {
    let mut v = solid_block(id, 13);
    v.set([12, 6, 6], material::AIR).unwrap();
    v.set([12, 7, 6], material::AIR).unwrap();
    v
}

/// Dense cuboid on negative coordinates with odd edges: every clipped
/// footprint is fully occupied, so the refined guard must read exactly zero
/// and let it coarsen (`div_euclid` correctness control).
fn solid_block_negative(id: &str, edge: i32) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).unwrap());
    for x in -edge..0 {
        for y in -edge..0 {
            for z in -edge..0 {
                v.set([x, y, z], STONE).unwrap();
            }
        }
    }
    v
}

fn thin_sheet(id: &str) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).unwrap());
    for x in 0..16 {
        for y in 0..32 {
            v.set([x, y, 0], STONE).unwrap();
        }
    }
    v
}

fn persp_at(origin_z: f32, distance: f32) -> Camera {
    Camera {
        eye_m: [0.25, 0.25, origin_z + distance],
        forward_m: [0.0, 0.0, -1.0],
        viewport_height_px: 1080.0,
        near_m: 0.1,
        projection: Projection::Perspective {
            vertical_fov_rad: FOV60,
        },
    }
}

fn ortho_at(origin_z: f32, distance: f32, view_height_m: f32) -> Camera {
    Camera {
        eye_m: [0.25, 0.25, origin_z + distance],
        forward_m: [0.0, 0.0, -1.0],
        viewport_height_px: 1080.0,
        near_m: 0.1,
        projection: Projection::Orthographic { view_height_m },
    }
}

fn lod_of(scene: &mut DetailScene, camera: &Camera, config: &LodConfig) -> Lod {
    scene.select_lods(camera, config).unwrap()[0].lod
}

#[test]
fn through_tunnel_is_held_at_source_far_away_perspective() {
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene.add_prototype(block_with_through_tunnel("tunnel")).unwrap();
    scene.place("t", "tunnel", Transform::identity()).unwrap();
    // Far enough that error budget alone would select Quarter for this size.
    assert_eq!(lod_of(&mut scene, &persp_at(1.0, 300.0), &config), Lod::Source);

    // Control: relaxing only the local guard lets the same geometry coarsen,
    // proving the guard (not distance or dilation) held it at Source.
    let relaxed = LodConfig {
        max_local_loss_fraction: 1.0,
        ..config
    };
    assert!(
        lod_of(&mut scene, &persp_at(1.0, 300.0), &relaxed) > Lod::Source,
        "relaxed local guard must allow coarsening"
    );
}

#[test]
fn through_tunnel_is_held_at_source_orthographic_zoomed_out() {
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene.add_prototype(block_with_through_tunnel("tunnel")).unwrap();
    scene.place("t", "tunnel", Transform::identity()).unwrap();
    assert_eq!(
        lod_of(&mut scene, &ortho_at(1.0, 10.0, 200.0), &config),
        Lod::Source
    );
    // Zooming in on the opening also resolves Source (consistent, not chatter).
    assert_eq!(
        lod_of(&mut scene, &ortho_at(1.0, 10.0, 4.0), &config),
        Lod::Source
    );
}

#[test]
fn boundary_face_pit_is_held_at_source_far_away() {
    // Blind spot: the pit lives in a boundary-partial coarse cell, so the
    // footprint-fully-inside guard reads 0 and dilation (~0.20 at Half)
    // passes the bias. Coarsening to Half deletes the pit faces.
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene.add_prototype(block_with_face_pit("pit")).unwrap();
    scene.place("p", "pit", Transform::identity()).unwrap();
    assert_eq!(
        lod_of(&mut scene, &persp_at(1.0, 300.0), &config),
        Lod::Source,
        "boundary face pit must hold Source"
    );
    assert_eq!(
        lod_of(&mut scene, &ortho_at(1.0, 10.0, 200.0), &config),
        Lod::Source,
        "boundary face pit must hold Source orthographically"
    );
    // Control: relaxing only the local guard recovers the unsafe coarse
    // choice, proving which guard held it.
    let relaxed = LodConfig {
        max_local_loss_fraction: 1.0,
        ..config
    };
    assert!(
        lod_of(&mut scene, &persp_at(1.0, 300.0), &relaxed) > Lod::Source,
        "relaxed local guard must allow coarsening"
    );
}

#[test]
fn unaligned_dense_cuboids_still_coarsen_including_negatives() {
    // Odd edges and negative coordinates produce boundary-partial coarse
    // cells, but every clipped footprint is fully occupied: loss reads exactly
    // zero and dense geometry must still coarsen. Edge 15 keeps Quarter
    // dilation (~0.18) under the bias so the control reaches Quarter. Thin
    // sheets stay held by the global dilation bias (clipped loss 0).
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene.add_prototype(solid_block("odd", 15)).unwrap();
    scene.add_prototype(solid_block_negative("neg", 15)).unwrap();
    scene.add_prototype(thin_sheet("sheet")).unwrap();
    scene.place("o", "odd", Transform::identity()).unwrap();
    scene
        .place(
            "n",
            "neg",
            Transform::new([-20.0, 0.0, 0.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    scene
        .place(
            "h",
            "sheet",
            Transform::new([20.0, 0.0, 0.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    for camera in [persp_at(1.0, 300.0), ortho_at(1.0, 10.0, 200.0)] {
        let selected = scene.select_lods(&camera, &config).unwrap();
        let at = |id: &str| selected.iter().find(|s| s.instance == id).unwrap().lod;
        assert_eq!(at("o"), Lod::Quarter, "odd-edge dense cuboid coarsens");
        assert_eq!(at("n"), Lod::Quarter, "negative dense cuboid coarsens");
        assert_eq!(at("h"), Lod::Source, "thin sheet still held");
    }
}

#[test]
fn thin_stem_and_sheet_stay_at_source_while_dense_block_coarsens() {
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene.add_prototype(solid_block("block", 8)).unwrap();
    scene.add_prototype(thin_stem("stem")).unwrap();
    scene.add_prototype(thin_sheet("sheet")).unwrap();
    scene.place("b", "block", Transform::identity()).unwrap();
    scene
        .place(
            "s",
            "stem",
            Transform::new([10.0, 0.0, 0.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    scene
        .place(
            "h",
            "sheet",
            Transform::new([20.0, 0.0, 0.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    let camera = persp_at(0.5, 300.0);
    let selected = scene.select_lods(&camera, &config).unwrap();
    let at = |id: &str| selected.iter().find(|s| s.instance == id).unwrap().lod;
    assert_eq!(at("b"), Lod::Quarter, "dense solid still coarsens");
    assert_eq!(at("s"), Lod::Source, "thin stem held at source");
    assert_eq!(at("h"), Lod::Source, "thin sheet held at source");
}

#[test]
fn adjacent_instances_coarsen_and_the_seam_stays_solid() {
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene.add_prototype(solid_block("block", 8)).unwrap();
    // Two 0.5 m cubes touching at the x = 0.5 m face: the seam is between
    // instances, never inside one prototype digest, so it must not trip the
    // local guard.
    scene.place("a", "block", Transform::identity()).unwrap();
    scene
        .place(
            "b",
            "block",
            Transform::new([0.5, 0.0, 0.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    let camera = persp_at(0.5, 300.0);
    let selected = scene.select_lods(&camera, &config).unwrap();
    assert_eq!(selected[0].lod, Lod::Quarter);
    assert_eq!(selected[1].lod, Lod::Quarter);
    // Authoritative collision is untouched by selection: the seam is solid.
    scene.prepare_batches(&camera, &config).unwrap();
    assert!(scene.is_collidable_world_metres([0.5, 0.1, 0.1]).unwrap());
    assert!(scene.is_collidable_world_metres([0.499, 0.1, 0.1]).unwrap());
    assert!(scene.is_collidable_world_metres([0.501, 0.1, 0.1]).unwrap());
}

#[test]
fn dense_block_coarsens_far_and_resolves_source_near_with_zoom() {
    let config = LodConfig::default();
    let dense = || {
        let mut scene = DetailScene::new();
        scene.add_prototype(solid_block("block", 8)).unwrap();
        scene.place("b", "block", Transform::identity()).unwrap();
        scene
    };
    assert_eq!(
        lod_of(&mut dense(), &persp_at(0.5, 300.0), &config),
        Lod::Quarter
    );
    assert_eq!(
        lod_of(&mut dense(), &persp_at(0.5, 1.0), &config),
        Lod::Source
    );
    assert_eq!(
        lod_of(&mut dense(), &ortho_at(0.5, 10.0, 200.0), &config),
        Lod::Quarter
    );
    assert_eq!(
        lod_of(&mut dense(), &ortho_at(0.5, 10.0, 4.0), &config),
        Lod::Source
    );
}

#[test]
fn selection_and_preparation_leave_authoritative_source_unchanged() {
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene.add_prototype(block_with_through_tunnel("tunnel")).unwrap();
    scene.add_prototype(solid_block("block", 8)).unwrap();
    scene.place("t", "tunnel", Transform::identity()).unwrap();
    scene
        .place(
            "b",
            "block",
            Transform::new([5.0, 0.0, 0.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    let before = scene
        .prototype("tunnel")
        .unwrap()
        .snapshot()
        .runs
        .len();
    let before_revision = scene.prototype("tunnel").unwrap().revision();
    let before_cells = scene.prototype("tunnel").unwrap().occupied_cells();
    for distance in [1.0, 90.0, 300.0] {
        scene
            .select_lods(&persp_at(1.0, distance), &config)
            .unwrap();
        scene
            .prepare_batches(&persp_at(1.0, distance), &config)
            .unwrap();
    }
    let after = scene.prototype("tunnel").unwrap();
    assert_eq!(after.snapshot().runs.len(), before);
    assert_eq!(after.revision(), before_revision);
    assert_eq!(after.occupied_cells(), before_cells);
    // The tunnel void is still authoritative air after every selection, and
    // the neighboring solid is still authoritative stone.
    let void_centre = [8.5 * SCALE_FINE_M, 8.5 * SCALE_FINE_M, 8.5 * SCALE_FINE_M];
    let solid_centre = [8.5 * SCALE_FINE_M, 8.5 * SCALE_FINE_M, 7.5 * SCALE_FINE_M];
    assert_eq!(scene.sample_world_metres(void_centre).unwrap(), None);
    assert!(!scene.is_collidable_world_metres(void_centre).unwrap());
    assert_eq!(
        scene.sample_world_metres(solid_centre).unwrap().map(|hit| hit.1),
        Some(STONE)
    );
    assert!(scene.is_collidable_world_metres(solid_centre).unwrap());
}

#[test]
fn local_loss_config_rejects_out_of_range_fractions() {
    let base = LodConfig::default();
    for bad in [
        LodConfig {
            max_local_loss_fraction: -0.1,
            ..base
        },
        LodConfig {
            max_local_loss_fraction: 1.5,
            ..base
        },
        LodConfig {
            max_local_loss_fraction: f32::NAN,
            ..base
        },
    ] {
        assert!(bad.validate().is_err());
    }
    LodConfig {
        max_local_loss_fraction: 1.0,
        ..base
    }
    .validate()
    .expect("1.0 allows any interior loss");
}
