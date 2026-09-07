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
        Self { direction_to_sun: [0.4, 0.85, 0.3], intensity: 0.8 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LightingSettings {
    pub sun: Sun,
    pub shadows: bool,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self { sun: Sun::default(), shadows: true }
    }
}

pub(crate) const SHADOW_HALF_EXTENT: f32 = 64.0;

pub(crate) struct ShadowCamera {
    pub view_proj: [[f32; 4]; 4],
    pub depth_span: f32,
    pub direction: [f32; 3],
}

impl ShadowCamera {
    pub fn new(eye: [f32; 3], sun: Sun, bounds: &[[[f32; 3]; 2]], size: u32) -> Result<Self, String> {
        let direction = Vec3::from_array(sun.direction_to_sun);
        let eye = Vec3::from_array(eye);
        if !eye.is_finite() || !direction.is_finite()
            || !direction.length_squared().is_finite() || direction.length_squared() < 1.0e-12
            || !sun.intensity.is_finite() || !(0.0..=16.0).contains(&sun.intensity)
        {
            return Err("Sun/eye must be finite; sun direction nonzero and intensity in 0..=16".into());
        }
        if ![1024, 2048].contains(&size) {
            return Err("Shadow map size must be 1024 or 2048".into());
        }
        let direction = direction.normalize();
        // A vertical sun needs an alternate up vector. Rotation depends only on the
        // light, never the view camera; turning the camera cannot pump shadow scale.
        let up = if direction.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
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
                let p = Vec3::new(aabb[corner & 1][0], aabb[(corner >> 1) & 1][1], aabb[(corner >> 2) & 1][2]);
                let z = light.transform_point3(p).z;
                if !z.is_finite() { return Err("Non-finite shadow caster bounds".into()); }
                min_z = min_z.min(z);
                max_z = max_z.max(z);
            }
        }
        min_z = ((min_z - 16.0) / 16.0).floor() * 16.0;
        max_z = ((max_z + 16.0) / 16.0).ceil() * 16.0;
        let projection = Mat4::orthographic_rh(x - SHADOW_HALF_EXTENT, x + SHADOW_HALF_EXTENT,
            y - SHADOW_HALF_EXTENT, y + SHADOW_HALF_EXTENT, -max_z, -min_z);
        let matrix = projection * light;
        if !matrix.is_finite() || !(max_z - min_z).is_finite() {
            return Err("Shadow projection exceeds finite range".into());
        }
        Ok(Self { view_proj: matrix.to_cols_array_2d(), depth_span: max_z - min_z, direction: direction.to_array() })
    }
}
