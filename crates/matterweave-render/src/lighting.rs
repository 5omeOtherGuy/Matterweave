//! Backend-independent sunlight settings and stable directional projection.
use glam::{Mat4, Vec3};

/// Direction points from a surface toward the sun; callers need not normalize it.
#[derive(Clone, Copy, Debug)]
pub struct Sun {
    pub direction_to_sun: [f32; 3],
    pub intensity: f32,
}

impl Default for Sun {
    fn default() -> Self {
        Self {
            direction_to_sun: [0.4, 0.85, 0.3],
            intensity: 0.8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LightingSettings {
    pub sun: Sun,
    pub shadows: bool,
    /// Supported sizes are 1024 and 2048. Changes retire resources after the frame fence.
    pub shadow_map_size: u32,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            sun: Sun::default(),
            shadows: true,
            shadow_map_size: 1024,
        }
    }
}

pub(crate) const SHADOW_HALF_EXTENT: f32 = 64.0;

pub(crate) struct ShadowCamera {
    pub view_proj: [[f32; 4]; 4],
    pub depth_span: f32,
    pub direction: [f32; 3],
}

impl ShadowCamera {
    pub fn new(
        eye: [f32; 3],
        sun: Sun,
        bounds: &[[[f32; 3]; 2]],
        size: u32,
    ) -> Result<Self, String> {
        let direction = Vec3::from_array(sun.direction_to_sun);
        let eye = Vec3::from_array(eye);
        if !eye.is_finite()
            || !direction.is_finite()
            || !direction.length_squared().is_finite()
            || direction.length_squared() < 1.0e-12
            || !sun.intensity.is_finite()
            || !(0.0..=16.0).contains(&sun.intensity)
        {
            return Err(
                "Sun/eye must be finite; sun direction nonzero and intensity in 0..=16".into(),
            );
        }
        if ![1024, 2048].contains(&size) {
            return Err("Shadow map size must be 1024 or 2048".into());
        }
        let direction = direction.normalize();
        // A vertical sun needs an alternate up vector. Rotation depends only on the
        // light, never the view camera; turning the camera cannot pump shadow scale.
        let up = if direction.y.abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let light = Mat4::look_to_rh(Vec3::ZERO, -direction, up);
        let center = light.transform_point3(eye);
        let texel = 2.0 * SHADOW_HALF_EXTENT / size as f32;
        let x = (center.x / texel).round() * texel;
        let y = (center.y / texel).round() * texel;
        // Include all resident caster heights, even outside the camera view. The
        // fallback also covers an empty scene; quantization avoids small Z shifts.
        let mut min_z = center.z - SHADOW_HALF_EXTENT;
        let mut max_z = center.z + SHADOW_HALF_EXTENT;
        for aabb in bounds {
            for corner in 0..8 {
                let p = Vec3::new(
                    aabb[corner & 1][0],
                    aabb[(corner >> 1) & 1][1],
                    aabb[(corner >> 2) & 1][2],
                );
                let z = light.transform_point3(p).z;
                if !z.is_finite() {
                    return Err("Non-finite shadow caster bounds".into());
                }
                min_z = min_z.min(z);
                max_z = max_z.max(z);
            }
        }
        min_z = ((min_z - 16.0) / 16.0).floor() * 16.0;
        max_z = ((max_z + 16.0) / 16.0).ceil() * 16.0;
        let projection = Mat4::orthographic_rh(
            x - SHADOW_HALF_EXTENT,
            x + SHADOW_HALF_EXTENT,
            y - SHADOW_HALF_EXTENT,
            y + SHADOW_HALF_EXTENT,
            -max_z,
            -min_z,
        );
        let matrix = projection * light;
        if !matrix.is_finite() || !(max_z - min_z).is_finite() {
            return Err("Shadow projection exceeds finite range".into());
        }
        Ok(Self {
            view_proj: matrix.to_cols_array_2d(),
            depth_span: max_z - min_z,
            direction: direction.to_array(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FLOOR: [[f32; 3]; 2] = [[-48., -1., -48.], [48., 0., 48.]];

    fn projected(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
    }

    #[test]
    fn camera_motion_below_a_texel_keeps_shadow_xy_fixed() {
        let sun = Sun {
            direction_to_sun: [0., 1., 0.],
            intensity: 0.8,
        };
        let a = ShadowCamera::new([0.; 3], sun, &[FLOOR], 2048).unwrap();
        let b = ShadowCamera::new([0.001, 0., 0.001], sun, &[FLOOR], 2048).unwrap();
        assert_eq!(
            projected(a.view_proj, [10., 0., 10.]),
            projected(b.view_proj, [10., 0., 10.])
        );
    }

    #[test]
    fn depth_covers_tall_offscreen_casters_in_front_of_receivers() {
        let tower = [[-1., 0., -1.], [1., 250., 1.]];
        let sun = Sun {
            direction_to_sun: [0., 1., 0.],
            intensity: 0.8,
        };
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
            let sun = Sun {
                direction_to_sun: direction,
                intensity: 0.8,
            };
            assert!(ShadowCamera::new([0.; 3], sun, &[FLOOR], 1024).is_err());
        }
        assert!(ShadowCamera::new([0.; 3], Sun::default(), &[FLOOR], 17).is_err());
        assert!(ShadowCamera::new([f32::NAN; 3], Sun::default(), &[], 1024).is_err());
        assert!(LightingSettings::default().shadows);
    }

    #[test]
    fn angled_sun_keeps_caster_and_receiver_on_the_same_shadow_ray() {
        let sun = Sun::default();
        let settings = LightingSettings::default();
        assert_eq!(settings.shadow_map_size, 1024);
        assert_eq!(settings.sun.intensity, 0.8);
        let camera = ShadowCamera::new([0.; 3], sun, &[FLOOR], 1024).unwrap();
        let receiver = [12., -1., -18.];
        let caster = std::array::from_fn(|i| receiver[i] + camera.direction[i] * 30.);
        let r = projected(camera.view_proj, receiver);
        let c = projected(camera.view_proj, caster);
        assert!((r[0] - c[0]).abs() < 1.0e-6 && (r[1] - c[1]).abs() < 1.0e-6);
        assert!(c[2] < r[2]);
    }

    #[test]
    fn empty_scene_and_all_axis_sun_directions_have_valid_projection() {
        for direction in [
            [1., 0., 0.],
            [-1., 0., 0.],
            [0., 1., 0.],
            [0., -1., 0.],
            [0., 0., 1.],
            [0., 0., -1.],
        ] {
            let sun = Sun {
                direction_to_sun: direction,
                intensity: 0.8,
            };
            let camera = ShadowCamera::new([0.; 3], sun, &[], 2048).unwrap();
            let p = projected(camera.view_proj, [0.; 3]);
            assert!(p.iter().all(|v| v.is_finite()) && p[2] > 0. && p[2] < 1.);
        }
    }
}
