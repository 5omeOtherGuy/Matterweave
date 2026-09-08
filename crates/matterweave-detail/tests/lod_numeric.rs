//! Numeric-representability contract for camera/config-derived LOD arithmetic.
//!
//! Every field is finite, yet derived values can still overflow or collapse:
//! an f32 squared forward norm overflows to infinity for components around
//! `1e30`, a subnormal FOV or view height with a normal viewport drives the
//! pixels-per-metre scale past `f32::MAX`, and a finite budget times a finite
//! hysteresis can exceed it too. An accepted camera must therefore give a
//! finite positive projection scale at the near plane and a nonzero finite
//! normalized forward; a config whose hysteresis thresholds leave `f32` must
//! be rejected. Selection on an accepted camera must see the true view-axis
//! depth (not everything collapsed onto the near plane) and a projected error
//! of exactly zero for the error-free `Source` level (never `0 * inf = NaN`).

use matterweave_detail::{Bounds, Camera, LodConfig, Projection};

fn persp(forward_m: [f32; 3], vertical_fov_rad: f32, viewport_height_px: f32) -> Camera {
    Camera {
        eye_m: [0.0, 0.0, 0.0],
        forward_m,
        viewport_height_px,
        near_m: 0.1,
        projection: Projection::Perspective { vertical_fov_rad },
    }
}

fn ortho(view_height_m: f32, viewport_height_px: f32) -> Camera {
    Camera {
        eye_m: [0.0, 0.0, 0.0],
        forward_m: [0.0, 0.0, 1.0],
        viewport_height_px,
        near_m: 0.1,
        projection: Projection::Orthographic { view_height_m },
    }
}

#[test]
fn huge_forward_components_keep_a_finite_normalized_depth() {
    // Each component is finite, but the f32 squared norm overflows to infinity,
    // so the old f32 normalization produced `inv = 0` and collapsed every
    // corner depth onto the near plane (or `0 * inf = NaN`).
    let camera = persp([1.0e30, 1.0e30, 0.0], 1.0, 1080.0);
    camera.validate().expect("finite forward must validate");
    let scale = camera.pixels_per_metre(camera.near_m);
    assert!(scale.is_finite() && scale > 0.0, "scale at near: {scale}");

    // Along the normalized forward [1/sqrt(2), 1/sqrt(2), 0], the nearest
    // corner (10, -1, -1)..(12, 1, 1) sits at depth (10 - 1) / sqrt(2).
    let bounds = Bounds {
        min: [10.0, -1.0, -1.0],
        max: [12.0, 1.0, 1.0],
    };
    let depth = camera.nearest_depth(&bounds);
    let expected = 9.0 / std::f32::consts::SQRT_2;
    assert!(
        (depth - expected).abs() < 1.0e-4,
        "depth {depth} != {expected}"
    );
    assert!(depth.is_finite() && depth >= camera.near_m);

    // The error-free Source level must project to exactly zero pixels, never
    // NaN, whatever the depth.
    let projected_source = camera.projected_error_px(0.0, depth);
    assert_eq!(projected_source, 0.0, "source projected error must be 0");
}

#[test]
fn projection_scale_overflow_at_near_plane_is_rejected() {
    // Subnormal vertical FOV: tan(fov/2) ~ 5e-41 drives the scale to ~1e44,
    // beyond f32::MAX, which made pixels_per_metre infinite and the projected
    // error of the Source level NaN (0 * inf).
    let subnormal_fov = persp([0.0, 0.0, 1.0], 1.0e-40, 1080.0);
    assert!(
        subnormal_fov.validate().is_err(),
        "subnormal FOV must not validate"
    );

    // Huge viewport with an ordinary FOV overflows the same way.
    let huge_viewport = persp([0.0, 0.0, 1.0], 1.0, 3.0e38);
    assert!(
        huge_viewport.validate().is_err(),
        "viewport that overflows the near-plane scale must not validate"
    );

    // Orthographic: subnormal view height against a normal viewport.
    let subnormal_height = ortho(1.0e-40, 1080.0);
    assert!(
        subnormal_height.validate().is_err(),
        "subnormal view height must not validate"
    );

    // Tiny-but-representable FOV stays accepted with a finite positive scale.
    let tiny_fov = persp([0.0, 0.0, 1.0], 1.0e-20, 1080.0);
    tiny_fov
        .validate()
        .expect("representable tiny FOV must validate");
    let scale = tiny_fov.pixels_per_metre(tiny_fov.near_m);
    assert!(scale.is_finite() && scale > 0.0, "scale at near: {scale}");
    assert_eq!(tiny_fov.projected_error_px(0.0, tiny_fov.near_m), 0.0);
}

#[test]
fn config_thresholds_that_leave_f32_are_rejected() {
    // Finite budget times finite hysteresis can still overflow to infinity,
    // which makes the switch-to-coarser threshold vacuous.
    let overflowing_product = LodConfig {
        error_budget_px: 3.0e38,
        hysteresis: 10.0,
        ..LodConfig::default()
    };
    assert!(
        overflowing_product.validate().is_err(),
        "budget * (1 + hysteresis) overflows f32 and must not validate"
    );

    let large_hysteresis_product = LodConfig {
        error_budget_px: 2.0,
        hysteresis: 3.0e38,
        ..LodConfig::default()
    };
    assert!(
        large_hysteresis_product.validate().is_err(),
        "budget times large hysteresis overflows f32 and must not validate"
    );

    // Large but representable thresholds stay accepted.
    let representable = LodConfig {
        error_budget_px: 1.0e38,
        hysteresis: 2.0,
        ..LodConfig::default()
    };
    representable
        .validate()
        .expect("representable thresholds must validate");
}
