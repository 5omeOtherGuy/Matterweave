//! Bounded, deterministic one-bounce diffuse surface irradiance reference.
//!
//! Each exposed unit voxel face owns one sample (no cross-wall interpolation).
//! Cosine-weighted hemisphere rays gather directly sunlit Lambertian surfaces:
//! cached value = E_indirect / pi = mean(albedo_hit * sun_intensity * cos_hit).
//! Sun intensity follows the world shader's E_sun / pi convention.
//! The world shader multiplies this by receiver albedo. Misses are black (no sky,
//! emissive or recursive bounce). Both segments use authoritative World DDA.
//! All world revisions invalidate the entire finite volume, including distant
//! occluders. Call update after every edit, even with a zero budget. Source epochs
//! must change when replacing a World, including replacements at equal revision.
//! No asynchronous jobs or mutable world snapshots exist in this reference path.
use crate::Sun;
use glam::Vec3;
use matterweave_core::World;

pub const MAX_FACE_SLOTS: usize = 24_576;
pub const MAX_UPDATE_RAYS: usize = 16_384;
pub const MAX_UPDATE_WORK: usize = 16_384;
const EPSILON: f32 = 0.001;
const NORMALS: [[i32; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

#[derive(Clone, Copy, Debug)]
pub struct UpdateBudget {
    /// At most this many DDA calls; clamped to MAX_UPDATE_RAYS. A sample reserves
    /// two rays, so a budget below two cannot advance an exposed face.
    pub rays: usize,
    /// At most this many face inspections/sample iterations; independently capped.
    pub work: usize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateStats {
    pub rays: usize,
    pub work: usize,
    pub complete: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    epoch: u64,
    revision: u64,
    sun: [f32; 4],
}
pub(crate) fn light_key(sun: Sun) -> Result<[f32; 4], String> {
    let d = Vec3::from_array(sun.direction_to_sun);
    if !d.is_finite()
        || !d.length_squared().is_finite()
        || d.length_squared() < 1e-12
        || !sun.intensity.is_finite()
        || !(0.0..=16.0).contains(&sun.intensity)
    {
        return Err("Indirect sun must be finite, nonzero, intensity in 0..=16".into());
    }
    let n = d.normalize();
    Ok([n.x, n.y, n.z, sun.intensity])
}

/// Fixed-size residency with fallible allocation after validation. Coordinates
/// are limited to +/-8192 so the face bias remains representable in f32.
/// Palette entries are linear diffuse reflectances, immutable and in [0,1].
/// Face order is +X,-X,+Y,-Y,+Z,-Z. Unit voxel geometry only: detail meshes and
/// dynamic mesh-only objects are not represented by World and are not supported.
pub struct IndirectVolume {
    pub(crate) origin: [i32; 3],
    pub(crate) dimensions: [u32; 3],
    pub(crate) values: Vec<[f32; 4]>,
    palette: [[f32; 3]; 256],
    samples: u32,
    distance: f32,
    key: Option<Key>,
    cursor: usize,
    sample_index: u32,
    sum: Vec3,
}
impl IndirectVolume {
    pub fn new(
        origin: [i32; 3],
        dimensions: [u32; 3],
        samples: u32,
        distance: f32,
        palette: [[f32; 3]; 256],
    ) -> Result<Self, String> {
        let slots = dimensions
            .iter()
            .try_fold(6usize, |n, &d| n.checked_mul(d as usize))
            .filter(|&n| n > 0 && n <= MAX_FACE_SLOTS)
            .ok_or("Indirect volume exceeds face residency cap")?;
        if !(1..=256).contains(&samples)
            || !distance.is_finite()
            || !(0.001..=256.).contains(&distance)
            || palette
                .iter()
                .flatten()
                .any(|&c| !c.is_finite() || !(0.0..=1.0).contains(&c))
        {
            return Err("Invalid indirect samples, range (0.001..=256), or linear palette".into());
        }
        for axis in 0..3 {
            let end = i64::from(origin[axis]) + i64::from(dimensions[axis]);
            if origin[axis] < -8192 || end > 8192 {
                return Err("Indirect bounds exceed precise voxel coordinate range".into());
            }
        }
        let mut values = Vec::new();
        values
            .try_reserve_exact(slots)
            .map_err(|e| format!("Indirect allocation: {e}"))?;
        values.resize(slots, [0.; 4]);
        Ok(Self {
            origin,
            dimensions,
            values,
            palette,
            samples,
            distance,
            key: None,
            cursor: 0,
            sample_index: 0,
            sum: Vec3::ZERO,
        })
    }
    pub fn resident_bytes(&self) -> usize {
        self.values.len() * 16 + std::mem::size_of::<Self>()
    }
    pub fn valid_for(&self, world: &World, epoch: u64, sun: Sun) -> bool {
        light_key(sun).is_ok_and(|sun| {
            self.key
                == Some(Key {
                    epoch,
                    revision: world.revision(),
                    sun,
                })
        })
    }
    pub(crate) fn cached_sun(&self) -> Option<[f32; 4]> {
        self.key.map(|k| k.sun)
    }
    pub(crate) fn source_valid(&self, world: &World, epoch: u64) -> bool {
        self.key
            .is_some_and(|k| k.epoch == epoch && k.revision == world.revision())
    }
    pub fn sample(&self, cell: [i32; 3], face: usize) -> [f32; 3] {
        if face >= 6 {
            return [0.; 3];
        }
        let c: [i64; 3] = std::array::from_fn(|a| i64::from(cell[a]) - i64::from(self.origin[a]));
        if (0..3).any(|a| c[a] < 0 || c[a] >= i64::from(self.dimensions[a])) {
            return [0.; 3];
        }
        let i = ((c[2] as usize * self.dimensions[1] as usize + c[1] as usize)
            * self.dimensions[0] as usize
            + c[0] as usize)
            * 6
            + face;
        let v = self.values[i];
        [v[0], v[1], v[2]]
    }
    /// An invalid input light also clears previous output. Work resumes only for
    /// the same source/light key; partially accumulated faces are never published.
    pub fn update(
        &mut self,
        world: &World,
        epoch: u64,
        sun: Sun,
        budget: UpdateBudget,
    ) -> Result<UpdateStats, String> {
        let light = light_key(sun);
        let key = light.as_ref().ok().map(|&sun| Key {
            epoch,
            revision: world.revision(),
            sun,
        });
        if key != self.key || key.is_none() {
            self.values.fill([0.; 4]);
            self.cursor = 0;
            self.sample_index = 0;
            self.sum = Vec3::ZERO;
            self.key = key;
        }
        let light = light?;
        let direction = Vec3::new(light[0], light[1], light[2]);
        let mut stats = UpdateStats::default();
        let ray_cap = budget.rays.min(MAX_UPDATE_RAYS);
        while self.cursor < self.values.len() && stats.work < budget.work.min(MAX_UPDATE_WORK) {
            stats.work += 1;
            let face = self.cursor % 6;
            let index = self.cursor / 6;
            let dx = self.dimensions[0] as usize;
            let dy = self.dimensions[1] as usize;
            let cell = [
                self.origin[0] + (index % dx) as i32,
                self.origin[1] + ((index / dx) % dy) as i32,
                self.origin[2] + (index / (dx * dy)) as i32,
            ];
            let normal = NORMALS[face];
            let neighbor = std::array::from_fn(|a| cell[a] + normal[a]);
            if world.get(cell) == 0 || world.get(neighbor) != 0 {
                self.cursor += 1;
                continue;
            }
            if stats.rays + 2 > ray_cap {
                break;
            }
            let n = Vec3::from_array(normal.map(|n| n as f32));
            let origin =
                Vec3::from_array(cell.map(|v| v as f32)) + Vec3::splat(0.5) + n * (0.5 + EPSILON);
            let ray = hemisphere(n, self.sample_index, self.samples);
            stats.rays += 1;
            if let Some(hit) = world.raycast(origin.to_array(), ray.to_array(), self.distance) {
                let hn = Vec3::from_array(hit.normal.map(|n| n as f32));
                let cosine = hn.dot(direction).max(0.);
                if cosine > 0. {
                    // Place the shadow origin outside the exact entry face.
                    let point = origin + ray * hit.distance + hn * EPSILON;
                    stats.rays += 1;
                    if world
                        .raycast(point.to_array(), direction.to_array(), self.distance)
                        .is_none()
                    {
                        self.sum += Vec3::from_array(self.palette[hit.material as usize])
                            * (light[3] * cosine);
                    }
                }
            }
            self.sample_index += 1;
            if self.sample_index == self.samples {
                let rgb = self.sum / self.samples as f32;
                self.values[self.cursor] = [rgb.x, rgb.y, rgb.z, 1.];
                self.cursor += 1;
                self.sample_index = 0;
                self.sum = Vec3::ZERO;
            }
        }
        stats.complete = self.cursor == self.values.len();
        Ok(stats)
    }
}
// Hammersley cosine-weighted quadrature. Runtime glam provides vector bases;
// std reverse_bits provides the radical inverse (no RNG dependency/state).
fn hemisphere(n: Vec3, i: u32, count: u32) -> Vec3 {
    let u = (i as f32 + 0.5) / count as f32;
    let phi = std::f32::consts::TAU * (i.reverse_bits() as f64 / 4294967296.0) as f32;
    let tangent = if n.y.abs() > 0.5 { Vec3::X } else { Vec3::Y };
    let bitangent = n.cross(tangent);
    tangent * (u.sqrt() * phi.cos()) + bitangent * (u.sqrt() * phi.sin()) + n * (1. - u).sqrt()
}
