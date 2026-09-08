//! Corrected-semantics tests demanded by lead review: perspective depth is the
//! nearest AABB depth along the view forward axis (not Euclidean eye distance),
//! zoom by field of view refines, and small deep openings are held at Source by
//! the local interior-loss guard (the global dilation bias alone could not).

use matterweave_detail::{
    material, Camera, DetailScene, DetailVolume, Lod, LodConfig, Projection, Scale, Transform, Yaw,
    SCALE_FINE_M,
};

const FOV60: f32 = std::f32::consts::PI / 3.0;

fn solid_block(id: &str, edge: i32, mat: u8) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).unwrap());
    for x in 0..edge {
        for y in 0..edge {
            for z in 0..edge {
                v.set([x, y, z], mat).unwrap();
            }
        }
    }
    v
}

/// A large solid cube (edge 16) with a single 1x1 tunnel bored 12 cells deep.
/// The removed volume is tiny relative to the whole, so the *global* dilation
/// fraction stays low and the guard does not hold it at Source.
fn solid_with_deep_pinhole(id: &str, mat: u8) -> DetailVolume {
    let mut v = solid_block(id, 16, mat);
    for x in 2..14 {
        v.set([x, 8, 8], material::AIR).unwrap();
    }
    v
}

/// Perspective camera at the origin looking down +Z.
fn eye_forward(eye: [f32; 3], fov: f32) -> Camera {
    Camera {
        eye_m: eye,
        forward_m: [0.0, 0.0, 1.0],
        viewport_height_px: 1080.0,
        near_m: 0.1,
        projection: Projection::Perspective {
            vertical_fov_rad: fov,
        },
    }
}

#[test]
fn perspective_depth_is_view_forward_not_euclidean_distance() {
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene
        .add_prototype(solid_block("block", 8, material::BANK_STONE))
        .unwrap();
    // Two instances at the SAME forward depth (z = 80) but different lateral
    // offsets, so their Euclidean eye distances differ substantially.
    scene
        .place(
            "ahead",
            "block",
            Transform::new([0.0, 0.0, 80.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    scene
        .place(
            "beside",
            "block",
            Transform::new([40.0, 0.0, 80.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    let camera = eye_forward([0.0, 0.0, 0.0], FOV60);
    let selected = scene.select_lods(&camera, &config).unwrap();
    let ahead = selected.iter().find(|s| s.instance == "ahead").unwrap();
    let beside = selected.iter().find(|s| s.instance == "beside").unwrap();

    // Same forward depth => same reported depth and same selected level. Euclidean
    // distance would report ~89 m for `beside` and coarsen it more aggressively.
    assert!(
        (ahead.depth_m - 80.0).abs() < 1.0,
        "ahead depth {}",
        ahead.depth_m
    );
    assert!(
        (beside.depth_m - 80.0).abs() < 1.0,
        "off-axis depth must be forward depth, got {}",
        beside.depth_m
    );
    assert_eq!(ahead.lod, beside.lod);
}

#[test]
fn narrowing_field_of_view_zooms_in_and_refines() {
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene
        .add_prototype(solid_block("block", 8, material::BANK_STONE))
        .unwrap();
    scene
        .place(
            "b",
            "block",
            Transform::new([0.0, 0.0, 200.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    // Wide FOV: the distant block is tiny -> coarse.
    let wide = scene
        .select_lods(&eye_forward([0.0, 0.0, 0.0], FOV60), &config)
        .unwrap()[0]
        .lod;
    // Narrow FOV (telephoto zoom) magnifies the same block -> finer.
    let narrow = scene
        .select_lods(
            &eye_forward([0.0, 0.0, 0.0], std::f32::consts::PI / 9.0),
            &config,
        )
        .unwrap()[0]
        .lod;
    assert_eq!(wide, Lod::Quarter);
    assert!(
        narrow < wide,
        "narrower fov must refine: {narrow:?} vs {wide:?}"
    );
}

#[test]
fn deep_pinhole_opening_is_held_at_source_by_the_local_loss_guard() {
    // The *global* dilation fraction of this opening is ~0.003, far below the
    // bias, but the local interior-loss guard sees the worst interior coarse
    // cell at 0.25 (factor 2) and holds the prototype at Source. Relaxing only
    // the local guard coarsens it again, proving which guard held it.
    let config = LodConfig::default();
    let mut scene = DetailScene::new();
    scene
        .add_prototype(solid_with_deep_pinhole("pin", material::BANK_STONE))
        .unwrap();
    scene
        .place(
            "p",
            "pin",
            Transform::new([0.0, 0.0, 400.0], Yaw::Deg0).unwrap(),
        )
        .unwrap();
    let far = scene
        .select_lods(&eye_forward([0.0, 0.0, 0.0], FOV60), &config)
        .unwrap()[0]
        .lod;
    assert_eq!(
        far,
        Lod::Source,
        "local interior-loss guard preserves the deep pinhole"
    );
    let relaxed = LodConfig {
        max_local_loss_fraction: 1.0,
        ..config
    };
    let coarse = scene
        .select_lods(&eye_forward([0.0, 0.0, 0.0], FOV60), &relaxed)
        .unwrap()[0]
        .lod;
    assert!(
        coarse > Lod::Source,
        "relaxed local guard coarsens: {coarse:?}"
    );
    // The quality cap remains an independent, unconditional hold.
    let capped = LodConfig {
        max_lod: Lod::Source,
        ..config
    };
    let held = scene
        .select_lods(&eye_forward([0.0, 0.0, 0.0], FOV60), &capped)
        .unwrap()[0]
        .lod;
    assert_eq!(held, Lod::Source);
}
