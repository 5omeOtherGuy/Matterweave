// Exercise projection behavior without requiring a window or GPU.
#[path = "../src/lighting.rs"]
mod lighting;
use lighting::{LightingSettings, ShadowCamera, Sun};

const FLOOR: [[f32; 3]; 2] = [[-48., -1., -48.], [48., 0., 48.]];

fn projected(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}

#[test]
fn camera_motion_below_a_texel_keeps_shadow_xy_fixed() {
    let sun = Sun { direction_to_sun: [0., 1., 0.], intensity: 0.8 };
    let a = ShadowCamera::new([0.; 3], sun, &[FLOOR], 2048).unwrap();
    let b = ShadowCamera::new([0.001, 0., 0.001], sun, &[FLOOR], 2048).unwrap();
    assert_eq!(projected(a.view_proj, [10., 0., 10.]), projected(b.view_proj, [10., 0., 10.]));
}

#[test]
fn depth_covers_tall_offscreen_casters_in_front_of_receivers() {
    let tower = [[-1., 0., -1.], [1., 250., 1.]];
    let sun = Sun { direction_to_sun: [0., 1., 0.], intensity: 0.8 };
    let camera = ShadowCamera::new([0.; 3], sun, &[FLOOR, tower], 1024).unwrap();
    let receiver = projected(camera.view_proj, [0., 0., 0.]);
    let caster = projected(camera.view_proj, [0., 250., 0.]);
    assert!(caster[2] > 0. && caster[2] < receiver[2] && receiver[2] < 1.);
    assert!(camera.depth_span >= 251.);
}

#[test]
fn negative_world_positions_remain_centered_and_finite() {
    let eye = [-230., -12., -230.];
    let camera = ShadowCamera::new(eye, Sun::default(), &[FLOOR], 1024).unwrap();
    let p = projected(camera.view_proj, eye);
    assert!(p[0].abs() < 0.002 && p[1].abs() < 0.002);
    assert!(camera.view_proj.iter().flatten().all(|v| v.is_finite()));
}

#[test]
fn invalid_lights_and_unsupported_map_sizes_are_rejected() {
    for direction in [[0.; 3], [f32::NAN, 1., 0.], [f32::INFINITY, 0., 0.]] {
        let sun = Sun { direction_to_sun: direction, intensity: 0.8 };
        assert!(ShadowCamera::new([0.; 3], sun, &[FLOOR], 1024).is_err());
    }
    assert!(ShadowCamera::new([0.; 3], Sun::default(), &[FLOOR], 17).is_err());
    assert!(ShadowCamera::new([f32::NAN; 3], Sun::default(), &[], 1024).is_err());
    assert!(LightingSettings::default().shadows);
}
