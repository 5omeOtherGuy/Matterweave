//! Opt-in, bounded, single-bounce specular reflection for the raster renderer.
//!
//! # Scope
//!
//! One ideal mirror bounce for one reflective fragment. There is no recursion, no
//! temporal history, no glossy/rough lobe, no denoiser and no hardware ray tracing.
//! Nonreflective rendering is the default and is bit-identical to the previous
//! output: with every mirror strength zero the shader skips the entire path.
//!
//! # Reflection convention
//!
//! With `p` the shaded world position, `n` the unit shading normal and `e` the eye:
//!
//! ```text
//! incident  d = normalize(p - e)                 (surface toward eye)
//! reflected r = d - 2 * dot(d, n) * n
//! origin    o = p + n * SURFACE_OFFSET           (outside the shaded face)
//! ```
//!
//! The single secondary ray `o + t*r` is traced through the bounded source volume
//! with a monotone grid DDA. `t` is limited by the volume exit, so no ray is
//! unbounded.
//!
//! # Termination of a missing ray
//!
//! A reflected ray terminates against the scene background/fog colour
//! [`BACKGROUND`] = `vec3(0.16, 0.24, 0.29)` at the distance where it leaves the
//! source volume. The same background colour is the fog target, so a miss and a
//! fully fogged hit agree. Rays that start inside a solid cell, start outside the
//! volume, leave through a zero-length interval or exhaust the trace budget
//! terminate the same way.
//!
//! # Shading of the reflected hit
//!
//! `albedo * (ambient(n) + max(dot(n, sun_dir), 0) * sun_intensity)` with
//! `ambient(n) = 0.28 + 0.12 * max(n.y, 0)`, matching `world.wgsl`. The reflected
//! hit receives **no** shadow-map lookup and no further bounce; this is a stated
//! limitation, not an oversight.
//!
//! # Source data and invalidation
//!
//! [`ReflectionVolume`] composes [`RayVolume`]: it reuses that packing, its
//! `+/-8192` bound, `64^3` cell cap and `(epoch, revision, seed)` validation, and
//! replaces the palette with one whose `.w` carries the per-material mirror
//! strength. Reuse reason: the material grid, indexing order and invalidation
//! contract are identical and already unit-tested; a second packer would be a
//! parallel implementation of the same thing with its own drift risk.
//!
//! A fragment's mirror strength is the mirror value of the *authoritative voxel
//! material* at `floor(p - n * SURFACE_OFFSET)`, read from the source volume. That
//! keeps material identity out of the vertex format and ties reflectivity to world
//! data rather than to mesh attributes; surfaces not represented by a covered voxel
//! (dynamic meshes, static instances, detail geometry) are nonreflective.
//!
//! Bound: at most one pending publication, no background job, one host-visible
//! coherent destination buffer per resource, and at most
//! `MAX_REFLECTION_CELLS * 4 + REFLECTION_PALETTE_BYTES` bytes uploaded.
use crate::ray_reference::RayVolume;
use crate::Sun;
use matterweave_core::World;

/// Experiment limit: the largest reflection source volume is 64³ cells.
pub const MAX_REFLECTION_AXIS: u32 = 64;
pub const MAX_REFLECTION_CELLS: usize = 64 * 64 * 64;
/// Configurable trace-step budget; 512 is the hard maximum.
pub const MAX_TRACE_STEPS: u32 = 512;
pub const MIN_TRACE_STEPS: u32 = 1;
/// Default: one plus the largest possible axis-crossing count of a 64³ volume, so
/// no ray inside the volume can exhaust the budget.
pub const DEFAULT_TRACE_STEPS: u32 = 64 + 64 + 64 + 1;
pub const REFLECTION_PALETTE_BYTES: usize = 256 * 16;
/// Bias moving the secondary-ray origin outside the shaded face. Must match the
/// value packed into `reflection_params.y`.
pub const SURFACE_OFFSET: f32 = 0.001;
/// Colour a reflected ray terminates against; identical to the fog colour.
pub const BACKGROUND: [f32; 3] = [0.16, 0.24, 0.29];
/// Distance fog density, mirroring `world.wgsl`.
pub const FOG_DENSITY: f32 = 0.013;

/// Per-material linear reflectance and mirror strength. Material zero is air and
/// is always nonreflective.
#[derive(Clone, Copy, Debug)]
pub struct MaterialTable {
    color: [[f32; 3]; 256],
    mirror: [f32; 256],
}

impl MaterialTable {
    /// Linear RGB in `0..=1`; mirror strengths default to zero (nonreflective).
    pub fn new(color: [[f32; 3]; 256]) -> Result<Self, String> {
        if color
            .iter()
            .flatten()
            .any(|&v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err("Reflection palette must be finite linear RGB in 0..=1".into());
        }
        Ok(Self {
            color,
            mirror: [0.; 256],
        })
    }
    /// `strength` is the mirror blend in `0..=1`; 1.0 is an ideal mirror.
    pub fn set_mirror(&mut self, material: u8, strength: f32) -> Result<(), String> {
        if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
            return Err("Mirror strength must be finite and in 0..=1".into());
        }
        self.mirror[material as usize] = strength;
        Ok(())
    }
    pub fn mirror(&self, material: u8) -> f32 {
        self.mirror[material as usize]
    }
    pub fn color(&self, material: u8) -> [f32; 3] {
        self.color[material as usize]
    }
    /// RGB reflectance of every material, for `RayVolume::pack`.
    pub fn colors(&self) -> [[f32; 3]; 256] {
        self.color
    }
    /// Group-0 binding 5: 256 vec4s, rgb reflectance plus mirror strength in `w`.
    pub fn palette(&self) -> [[f32; 4]; 256] {
        std::array::from_fn(|i| {
            let c = self.color[i];
            [c[0], c[1], c[2], self.mirror[i]]
        })
    }
    pub fn any_mirror(&self) -> bool {
        self.mirror.iter().any(|&m| m > 0.)
    }
}

/// Logical payload sizes, not allocator overhead or driver-side residency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReflectionMemoryStats {
    /// Group-0 binding 4: one u32 per cell.
    pub material_bytes: usize,
    /// Group-0 binding 5: 256 vec4s.
    pub palette_bytes: usize,
    /// One complete upload of both descriptors.
    pub upload_bytes: usize,
    /// CPU-side resident payload including the packed grid copy.
    pub resident_bytes: usize,
}

/// One reflected ray's result in the CPU oracle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionSample {
    pub hit: bool,
    pub cell: [i32; 3],
    pub normal: [i32; 3],
    /// Distance from the shaded point; on a miss the volume-exit distance.
    pub distance: f32,
    pub material: u8,
}

impl ReflectionSample {
    /// Terminated against the background at `distance`.
    pub fn miss(distance: f32) -> Self {
        Self {
            hit: false,
            cell: [0; 3],
            normal: [0; 3],
            distance,
            material: 0,
        }
    }
}

/// Immutable derived snapshot of a half-open integer AABB, at most 64³ cells.
/// Everything outside it is intentionally excluded: material zero is air, every
/// other ID is solid. Callers MUST change `epoch` when replacing a `World` even
/// at an identical revision, and validity additionally re-derives a footprint
/// digest so a replaced scene with an unchanged revision cannot publish.
#[derive(Debug)]
pub struct ReflectionVolume {
    pack: RayVolume,
    palette: [[f32; 4]; 256],
    trace_steps: u32,
    digest: u64,
}

impl ReflectionVolume {
    /// Validates before allocation. Each axis is in `1..=64` and the product is at
    /// most `64³`; endpoints must lie within `+/-8192`.
    pub fn pack(
        world: &World,
        epoch: u64,
        origin: [i32; 3],
        dimensions: [u32; 3],
        materials: &MaterialTable,
        trace_steps: u32,
    ) -> Result<Self, String> {
        if dimensions
            .iter()
            .any(|&d| d == 0 || d > MAX_REFLECTION_AXIS)
        {
            return Err(format!(
                "Reflection dimensions must be in 1..={MAX_REFLECTION_AXIS}"
            ));
        }
        if !(MIN_TRACE_STEPS..=MAX_TRACE_STEPS).contains(&trace_steps) {
            return Err(format!(
                "Reflection trace steps must be in {MIN_TRACE_STEPS}..={MAX_TRACE_STEPS}"
            ));
        }
        let pack = RayVolume::pack(world, epoch, origin, dimensions, materials.colors())?;
        Ok(Self {
            pack,
            palette: materials.palette(),
            trace_steps,
            digest: footprint_digest(world, origin, dimensions),
        })
    }
    pub fn origin(&self) -> [i32; 3] {
        self.pack.origin()
    }
    pub fn dimensions(&self) -> [u32; 3] {
        self.pack.dimensions()
    }
    /// Group 0 binding 4: `x + dims.x * (y + dims.y * z)`, u32 stride 4.
    pub fn materials(&self) -> &[u32] {
        self.pack.materials()
    }
    /// Group 0 binding 5: exactly 256 vec4s, stride 16, mirror strength in `w`.
    pub fn palette(&self) -> &[[f32; 4]; 256] {
        &self.palette
    }
    pub fn trace_steps(&self) -> u32 {
        self.trace_steps
    }
    /// One plus the largest possible axis-crossing count inside this volume.
    pub fn crossing_bound(&self) -> u32 {
        self.dimensions().iter().sum::<u32>() + 1
    }
    /// Effective WGSL iteration bound: the configured budget never exceeds the
    /// geometric crossing bound, so a generous budget cannot lengthen any ray.
    pub fn trace_bound(&self) -> u32 {
        self.trace_steps.min(self.crossing_bound())
    }
    pub fn source_epoch(&self) -> u64 {
        self.pack.source_epoch()
    }
    pub fn source_revision(&self) -> u64 {
        self.pack.source_revision()
    }
    pub fn source_seed(&self) -> u64 {
        self.pack.source_seed()
    }
    /// Conservative: even edits outside the crop invalidate this pack, and the
    /// footprint digest is recomputed so an equal-revision replacement cannot be
    /// accepted. Cost is one `World::get` per cell (at most 262,144), incurred only
    /// at publication, never per frame.
    pub fn valid_for(&self, world: &World, epoch: u64) -> bool {
        self.pack.valid_for(world, epoch)
            && self.digest == footprint_digest(world, self.origin(), self.dimensions())
    }
    /// Digest of the authoritative source at publication time.
    pub fn source_digest(&self) -> u64 {
        self.digest
    }
    /// Material stored in the packed grid, or zero outside it.
    pub fn material_at(&self, cell: [i32; 3]) -> u8 {
        let d = self.dimensions();
        let local: [i32; 3] = std::array::from_fn(|a| cell[a] - self.origin()[a]);
        if (0..3).any(|a| local[a] < 0 || local[a] >= d[a] as i32) {
            return 0;
        }
        let index = (local[0] as usize)
            + d[0] as usize * (local[1] as usize + d[1] as usize * local[2] as usize);
        u8::try_from(self.materials()[index]).unwrap_or(u8::MAX)
    }
    pub fn mirror_at(&self, cell: [i32; 3]) -> f32 {
        self.palette()[self.material_at(cell) as usize][3]
    }
    pub fn memory_stats(&self) -> ReflectionMemoryStats {
        let material_bytes = self.materials().len() * size_of::<u32>();
        ReflectionMemoryStats {
            material_bytes,
            palette_bytes: REFLECTION_PALETTE_BYTES,
            upload_bytes: material_bytes + REFLECTION_PALETTE_BYTES,
            resident_bytes: material_bytes + REFLECTION_PALETTE_BYTES + size_of::<Self>(),
        }
    }
}

/// FNV-1a over the authoritative material of every cell in the footprint, in the
/// same x-fastest order as the packed grid. Independent of the packing code so it
/// can detect a replaced scene whose revision and seed are unchanged.
pub fn footprint_digest(world: &World, origin: [i32; 3], dimensions: [u32; 3]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for z in 0..dimensions[2] {
        for y in 0..dimensions[1] {
            for x in 0..dimensions[0] {
                hash ^= u64::from(world.get([
                    origin[0] + x as i32,
                    origin[1] + y as i32,
                    origin[2] + z as i32,
                ]));
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

/// Reflected direction of `incident` about unit `normal`; zero for invalid input.
pub fn reflect_direction(incident: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let i = match glam::Vec3::from_array(incident) {
        v if v.is_finite() => v,
        _ => return [0.; 3],
    };
    let n = match glam::Vec3::from_array(normal) {
        v if v.is_finite() => v,
        _ => return [0.; 3],
    };
    if i.length_squared() < 1e-12 || n.length_squared() < 1e-12 {
        return [0.; 3];
    }
    (i.normalize() - 2. * i.normalize().dot(n.normalize()) * n.normalize()).to_array()
}

/// Secondary-ray origin for a shaded point, biased outside the shaded face.
pub fn surface_origin(point: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|a| point[a] + normal[a] * SURFACE_OFFSET)
}

/// The start cell of the secondary DDA: the shaded face's solid voxel.
pub fn surface_cell(point: [f32; 3], normal: [f32; 3]) -> [i32; 3] {
    let p = glam::Vec3::from_array(point) - glam::Vec3::from_array(normal) * SURFACE_OFFSET;
    std::array::from_fn(|a| p[a].floor() as i32)
}

/// Independent CPU oracle for one reflected ray.
///
/// It slab-clips the ray to the source volume and then uses the authoritative
/// [`World::raycast`] DDA, never the shader's traversal. Within the volume the
/// packed grid and the world agree by construction, so an agreement test is a
/// genuine cross-check rather than a mirror of the implementation.
pub fn reflect_sample(
    volume: &ReflectionVolume,
    world: &World,
    point: [f32; 3],
    normal: [f32; 3],
    eye: [f32; 3],
) -> ReflectionSample {
    let n = glam::Vec3::from_array(normal);
    let p = glam::Vec3::from_array(point);
    let e = glam::Vec3::from_array(eye);
    if !n.is_finite() || !p.is_finite() || !e.is_finite() || n.length_squared() < 1e-12 {
        return ReflectionSample::miss(0.);
    }
    let n = n.normalize();
    let incident = p - e;
    if incident.length_squared() < 1e-12 {
        return ReflectionSample::miss(0.);
    }
    let incident = incident.normalize();
    let direction = incident - 2. * incident.dot(n) * n;
    if !direction.is_finite() || direction.length_squared() < 1e-12 {
        return ReflectionSample::miss(0.);
    }
    let origin = p + n * SURFACE_OFFSET;
    let lower = glam::Vec3::from_array(volume.origin().map(|v| v as f32));
    let upper = lower + glam::Vec3::from_array(volume.dimensions().map(|v| v as f32));
    // A start outside the half-open volume terminates immediately.
    if (0..3).any(|a| origin[a] < lower[a] || origin[a] >= upper[a]) {
        return ReflectionSample::miss(0.);
    }
    let mut exit = f32::INFINITY;
    for axis in 0..3 {
        if direction[axis] > 0. {
            exit = exit.min((upper[axis] - origin[axis]) / direction[axis]);
        } else if direction[axis] < 0. {
            exit = exit.min((lower[axis] - origin[axis]) / direction[axis]);
        }
    }
    if !exit.is_finite() || !(exit > 0.) {
        return ReflectionSample::miss(0.);
    }
    let start = std::array::from_fn(|a| origin[a].floor() as i32);
    // Self-intersection: a solid start cell would report a zero-distance hit on
    // the surface being shaded.
    if world.get(start) != 0 {
        return ReflectionSample::miss(exit);
    }
    let Some(hit) = world.raycast(origin.to_array(), direction.to_array(), exit) else {
        return ReflectionSample::miss(exit);
    };
    // A monotone DDA inspects the start cell plus one cell per crossed plane.
    let needed = 1u32
        + (0..3)
            .map(|a| (hit.cell[a] - start[a]).unsigned_abs())
            .sum::<u32>();
    if needed > volume.trace_bound() {
        return ReflectionSample::miss(exit);
    }
    ReflectionSample {
        hit: true,
        cell: hit.cell,
        normal: hit.normal,
        distance: hit.distance,
        material: hit.material,
    }
}

/// Matched simple shading of a reflection sample, mirroring `world.wgsl`:
/// albedo * (ambient + sun term) on a hit, [`BACKGROUND`] on a miss. No shadow
/// lookup and no further bounce is applied to the reflected hit.
pub fn shade_sample(
    volume: &ReflectionVolume,
    sample: &ReflectionSample,
    sun: Sun,
) -> Result<[f32; 3], String> {
    let light = crate::indirect::light_key(sun)?;
    if !sample.hit {
        return Ok(BACKGROUND);
    }
    let albedo = volume.palette()[sample.material as usize];
    let albedo = glam::Vec3::new(albedo[0], albedo[1], albedo[2]);
    let n = glam::Vec3::from_array(sample.normal.map(|v| v as f32));
    let ambient = 0.28 + 0.12 * n.y.max(0.);
    let sunlight = if n == glam::Vec3::ZERO {
        0.
    } else {
        glam::Vec3::from_array([light[0], light[1], light[2]])
            .dot(n.normalize())
            .max(0.)
            * light[3]
    };
    Ok((albedo * (ambient + sunlight)).to_array())
}

/// Distance fog applied by `world.wgsl` to the total view path length.
pub fn fog_mix(color: [f32; 3], path_length: f32) -> [f32; 3] {
    let fog = 1. - (-path_length.max(0.) * FOG_DENSITY).exp();
    glam::Vec3::from_array(color)
        .lerp(glam::Vec3::from_array(BACKGROUND), fog)
        .to_array()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    const MIRROR: u8 = 7;
    const WALL: u8 = 2;
    const TARGET: u8 = 1;

    fn table() -> MaterialTable {
        let mut color = [[0.5; 3]; 256];
        color[0] = [0.; 3];
        color[MIRROR as usize] = [0.42, 0.77, 0.72];
        color[WALL as usize] = [0.65; 3];
        color[TARGET as usize] = [0.9, 0.05, 0.02];
        let mut t = MaterialTable::new(color).unwrap();
        t.set_mirror(MIRROR, 1.0).unwrap();
        t
    }

    /// Mirror floor of material [`MIRROR`] at cell `y = -1`, plus an optional full
    /// slab at `x = wall_x` spanning `y` in `0..8` and the whole `z` footprint.
    /// A slab is used because its entry cell is predictable: any ray travelling
    /// toward it enters at `x = wall_x`.
    fn scene(floor: std::ops::Range<i32>, wall: Option<(i32, u8)>) -> World {
        let mut w = World::new(31);
        for x in floor.clone() {
            for z in floor.clone() {
                w.set([x, -1, z], MIRROR);
            }
        }
        if let Some((x, material)) = wall {
            for z in floor.clone() {
                for y in 0..8 {
                    w.set([x, y, z], material);
                }
            }
        }
        w
    }

    fn volume(world: &World, steps: u32) -> ReflectionVolume {
        ReflectionVolume::pack(world, 0, [-16, -8, -16], [32, 24, 32], &table(), steps).unwrap()
    }

    fn negative_volume(world: &World, steps: u32) -> ReflectionVolume {
        ReflectionVolume::pack(world, 0, [-24, -8, -24], [16, 24, 16], &table(), steps).unwrap()
    }

    /// Reflected direction computed directly from the documented convention, not
    /// through `reflect_direction`, so the oracle is not used to check itself.
    fn reflected(point: [f32; 3], normal: [f32; 3], eye: [f32; 3]) -> Vec3 {
        let d = (Vec3::from_array(point) - Vec3::from_array(eye)).normalize();
        let n = Vec3::from_array(normal).normalize();
        d - 2. * d.dot(n) * n
    }

    #[test]
    fn packs_mirror_palette_and_bounds() {
        let world = scene(-12..12, None);
        let v = volume(&world, DEFAULT_TRACE_STEPS);
        assert_eq!(v.dimensions(), [32, 24, 32]);
        assert_eq!(v.origin(), [-16, -8, -16]);
        assert_eq!(v.palette()[MIRROR as usize][3], 1.0);
        assert_eq!(v.palette()[WALL as usize][3], 0.0);
        assert_eq!(v.material_at([0, -1, 0]), MIRROR);
        assert_eq!(v.material_at([0, 40, 0]), 0);
        assert_eq!(v.mirror_at([0, -1, 0]), 1.0);
        assert_eq!(v.mirror_at([0, 0, 0]), 0.0);
        assert_eq!(v.crossing_bound(), 32 + 24 + 32 + 1);
        assert_eq!(
            v.trace_bound(),
            v.crossing_bound(),
            "the default budget is clamped to the geometric crossing bound"
        );
        assert_eq!(v.source_seed(), 31);
        let stats = v.memory_stats();
        assert_eq!(stats.material_bytes, 32 * 24 * 32 * 4);
        assert_eq!(stats.palette_bytes, REFLECTION_PALETTE_BYTES);
        assert_eq!(
            stats.upload_bytes,
            stats.material_bytes + REFLECTION_PALETTE_BYTES
        );
        assert!(stats.resident_bytes >= stats.upload_bytes);
        assert!(v.valid_for(&world, 0));
        assert!(!v.valid_for(&world, 1));
    }

    #[test]
    fn rejects_oversized_volumes_invalid_steps_and_palettes() {
        let world = World::new(0);
        let t = table();
        for dims in [
            [0, 1, 1],
            [65, 1, 1],
            [64, 64, 65],
            [u32::MAX, 1, 1],
            [65; 3],
        ] {
            assert!(ReflectionVolume::pack(&world, 0, [0; 3], dims, &t, 64).is_err());
        }
        for steps in [0, 513, u32::MAX] {
            assert!(ReflectionVolume::pack(&world, 0, [0; 3], [1; 3], &t, steps).is_err());
        }
        for steps in [MIN_TRACE_STEPS, MAX_TRACE_STEPS] {
            assert!(ReflectionVolume::pack(&world, 0, [0; 3], [1; 3], &t, steps).is_ok());
        }
        // A 64³ volume is exactly at the experiment cap and is accepted.
        let big =
            ReflectionVolume::pack(&world, 0, [-32; 3], [64; 3], &t, MAX_TRACE_STEPS).unwrap();
        assert_eq!(big.materials().len(), MAX_REFLECTION_CELLS);
        assert_eq!(big.memory_stats().material_bytes, 4 * MAX_REFLECTION_CELLS);
        assert_eq!(big.crossing_bound(), 193);
        assert_eq!(big.trace_bound(), 193, "budget cannot exceed crossings");
        for invalid in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            let mut colors = t.colors();
            colors[9][1] = invalid;
            assert!(MaterialTable::new(colors).is_err());
            let mut bad = table();
            assert!(bad.set_mirror(3, invalid).is_err());
        }
        assert!(table().set_mirror(3, 0.5).is_ok());
        assert!(table().any_mirror());
    }

    #[test]
    fn empty_volume_and_maximum_bounds_terminate_as_misses() {
        let world = World::new(3);
        let v = ReflectionVolume::pack(&world, 0, [-4; 3], [8; 3], &table(), 193).unwrap();
        assert!(v.materials().iter().all(|&m| m == 0));
        let sample = reflect_sample(&v, &world, [0.5, 0., 0.5], [0., 1., 0.], [4., 3., 4.]);
        assert!(!sample.hit);
        assert!(
            sample.distance > 0.,
            "a miss still reports its exit distance"
        );
        assert_eq!(
            shade_sample(&v, &sample, Sun::default()).unwrap(),
            BACKGROUND,
            "a missing ray terminates against the background"
        );
        // A volume at the +/-8192 coordinate bound packs and misses cleanly.
        let edge =
            ReflectionVolume::pack(&world, 0, [8192 - 64; 3], [64; 3], &table(), 512).unwrap();
        assert_eq!(edge.origin(), [8192 - 64; 3]);
        assert!(edge.materials().iter().all(|&m| m == 0));
        assert!(!reflect_sample(&edge, &world, [0., 0., 0.], [0., 1., 0.], [1., 4., 1.]).hit);
    }

    #[test]
    fn oblique_reflection_hits_a_slab_at_the_predicted_entry_and_distance() {
        let world = scene(-12..12, Some((5, WALL)));
        let v = volume(&world, DEFAULT_TRACE_STEPS);
        let point = [0.5, 0.0, 0.5];
        let normal = [0., 1., 0.];
        let eye = [-6., 5., 0.5];
        let sample = reflect_sample(&v, &world, point, normal, eye);
        assert!(sample.hit);
        assert_eq!(sample.material, WALL);
        assert_eq!(sample.cell[0], 5, "a slab is entered at its first plane");
        assert_eq!(sample.cell[2], 0, "the z component stays inside one cell");
        assert_eq!(sample.normal, [-1, 0, 0], "entering +X exposes the -X face");
        let r = reflected(point, normal, eye);
        let origin = Vec3::from_array(surface_origin(point, normal));
        let expected = (5.0 - origin.x) / r.x;
        assert!(
            (sample.distance - expected).abs() < 1e-3,
            "distance {} vs analytic {}",
            sample.distance,
            expected
        );
        // The same surface viewed straight down the normal reflects straight back.
        let straight = reflect_sample(&v, &world, point, normal, [0.5, 6., 0.5]);
        assert!(!straight.hit, "a normal-incidence ray leaves the volume");
    }

    #[test]
    fn negative_coordinates_and_grazing_rays_use_the_same_contract() {
        let world = scene(-22..-10, Some((-19, WALL)));
        let v = negative_volume(&world, DEFAULT_TRACE_STEPS);
        let point = [-20.5, 0.0, -20.5];
        let normal = [0., 1., 0.];
        // Eye to -X and above: the reflection travels +X into the negative slab.
        let sample = reflect_sample(&v, &world, point, normal, [-26., 5., -20.5]);
        assert!(sample.hit, "negative coordinates must not be special-cased");
        assert_eq!(sample.material, WALL);
        assert_eq!(sample.cell[0], -19);
        assert_eq!(sample.normal, [-1, 0, 0]);
        assert!(sample.cell[1] >= 0 && sample.cell[1] < 8);
        // A grazing ray travelling away from the slab stays in the empty air row
        // above the floor and leaves the volume without hitting anything.
        let grazing = reflect_sample(&v, &world, point, normal, [-14., 0.35, -20.5]);
        assert!(!grazing.hit, "a grazing ray leaves through the far wall");
        assert!(grazing.distance > 0.);
        assert!(
            (Vec3::from_array(reflected(point, normal, [-14., 0.35, -20.5]).to_array())[0]) < 0.,
            "the grazing ray travels -X, away from the slab"
        );
    }

    #[test]
    fn parallel_rays_and_volume_shell_faces_are_handled() {
        // Two cells tall and one cell thick in Z: the reflected ray runs parallel
        // to four volume faces, including both faces of the thin Z axis.
        let mut world = World::new(12);
        for x in 0..8 {
            world.set([x, -1, 0], MIRROR);
        }
        world.set([4, 0, 0], TARGET);
        let v = ReflectionVolume::pack(&world, 0, [0, -1, 0], [8, 2, 1], &table(), 193).unwrap();
        assert_eq!(v.dimensions(), [8, 2, 1]);
        // A shallow oblique view keeps the ray inside the single air cell row.
        let sample = reflect_sample(&v, &world, [0.5, 0.0, 0.5], [0., 1., 0.], [-2., 0.35, 0.5]);
        assert!(sample.hit);
        assert_eq!(sample.cell, [4, 0, 0]);
        assert_eq!(sample.material, TARGET);
        assert_eq!(sample.normal, [-1, 0, 0]);
        // An outward-facing face on the volume shell starts outside: a miss.
        for (point, normal) in [
            ([0.5, 1.0, 0.5], [0., 1., 0.]),
            ([0.5, -1.0, 0.5], [0., -1., 0.]),
            ([0.5, 0.0, 1.5], [0., 0., 1.]),
        ] {
            let outside = reflect_sample(&v, &world, point, normal, [-2., 0.35, 0.5]);
            assert!(!outside.hit, "shell face {point:?} {normal:?} must miss");
        }
    }

    #[test]
    fn self_intersection_and_origin_inside_solid_terminate() {
        let mut world = World::new(7);
        world.set([0, 0, 0], MIRROR);
        world.set([0, 1, 0], WALL); // solid neighbour above the shaded face
        let v = ReflectionVolume::pack(&world, 0, [-2; 3], [6; 3], &table(), 193).unwrap();
        let sample = reflect_sample(&v, &world, [0.5, 1.0, 0.5], [0., 1., 0.], [3., 6., 3.]);
        assert!(
            !sample.hit,
            "an origin inside a solid cell must not self-hit"
        );
        assert!(sample.distance > 0.);
        // A normal floor face: the offset origin is in the air neighbour.
        let open = scene(-4..4, None);
        let v =
            ReflectionVolume::pack(&open, 0, [-6, -8, -6], [12, 24, 12], &table(), 193).unwrap();
        let sample = reflect_sample(&v, &open, [0.5, 0.0, 0.5], [0., 1., 0.], [6., 5., 6.]);
        assert!(!sample.hit, "empty space above the floor is a miss");
        assert!(sample.distance > 0.);
    }

    #[test]
    fn trace_budget_exhaustion_is_a_declared_miss() {
        let world = scene(-12..12, Some((5, WALL)));
        let point = [0.5, 0.0, 0.5];
        let normal = [0., 1., 0.];
        let eye = [-6., 5., 0.5];
        let full = volume(&world, DEFAULT_TRACE_STEPS);
        let hit = reflect_sample(&full, &world, point, normal, eye);
        assert!(hit.hit);
        let start = [0, 0, 0];
        let needed = 1
            + (0..3)
                .map(|a| (hit.cell[a] - start[a]).unsigned_abs())
                .sum::<u32>();
        let starved = volume(&world, needed - 1);
        assert_eq!(starved.trace_bound(), needed - 1);
        assert!(
            !reflect_sample(&starved, &world, point, normal, eye).hit,
            "an exhausted budget must terminate as a miss"
        );
        let exact = volume(&world, needed);
        assert!(reflect_sample(&exact, &world, point, normal, eye).hit);
        // Generous budgets are clamped to the geometric crossing bound.
        let huge = volume(&world, MAX_TRACE_STEPS);
        assert_eq!(huge.trace_bound(), huge.crossing_bound());
        assert!(reflect_sample(&huge, &world, point, normal, eye).hit);
    }

    #[test]
    fn invalid_input_never_hits() {
        let world = scene(-4..4, None);
        let v = ReflectionVolume::pack(&world, 0, [-6; 3], [12; 3], &table(), 193).unwrap();
        for (point, normal, eye) in [
            ([0.5, 0., 0.5], [0.; 3], [1., 1., 1.]),
            ([0.5, 0., 0.5], [f32::NAN, 1., 0.], [1., 1., 1.]),
            ([f32::INFINITY, 0., 0.5], [0., 1., 0.], [1., 1., 1.]),
            ([0.5, 0., 0.5], [0., 1., 0.], [0.5, 0., 0.5]),
            ([0.5, 0., 0.5], [0., 1., 0.], [f32::NAN; 3]),
        ] {
            assert!(!reflect_sample(&v, &world, point, normal, eye).hit);
        }
        assert_eq!(reflect_direction([1., 0., 0.], [0.; 3]), [0.; 3]);
        assert_eq!(reflect_direction([0.; 3], [0., 1., 0.]), [0.; 3]);
        let up = reflect_direction([0., -1., 0.], [0., 1., 0.]);
        assert!((up[1] - 1.).abs() < 1e-6);
        assert_eq!(
            shade_sample(&v, &ReflectionSample::miss(1.), Sun::default()).unwrap(),
            BACKGROUND
        );
        assert!(shade_sample(
            &v,
            &ReflectionSample::miss(1.),
            Sun {
                direction_to_sun: [0.; 3],
                intensity: 1.
            }
        )
        .is_err());
    }

    #[test]
    fn delayed_result_cannot_publish_after_edit_or_replacement() {
        let mut world = World::new(11);
        world.set([0, 0, 0], TARGET);
        let t = table();
        let prepared = ReflectionVolume::pack(&world, 3, [-2; 3], [5; 3], &t, 193).unwrap();
        let revision = world.revision();
        world.set([0, 0, 0], 0); // state B
        world.set([0, 0, 0], TARGET); // state A again, at a new revision
        assert!(world.revision() > revision);
        assert!(
            !prepared.valid_for(&world, 3),
            "an A->B->A round trip must not reuse a stale pack"
        );
        // Scene replacement at an equal revision: only the epoch distinguishes them.
        let mut other = World::new(11);
        other.set([1, 1, 1], TARGET);
        assert_eq!(other.revision(), 1);
        let mut same_revision = World::new(11);
        same_revision.set([2, 2, 2], TARGET);
        assert_eq!(same_revision.revision(), other.revision());
        let fresh = ReflectionVolume::pack(&same_revision, 3, [-2; 3], [5; 3], &t, 193).unwrap();
        assert!(fresh.valid_for(&same_revision, 3));
        assert_eq!(
            fresh.source_digest(),
            footprint_digest(&same_revision, [-2; 3], [5; 3])
        );
        assert!(
            !fresh.valid_for(&other, 3),
            "equal revision and seed must not validate a different scene"
        );
        assert!(fresh.source_digest() != footprint_digest(&other, [-2; 3], [5; 3]));
        assert!(!fresh.valid_for(&same_revision, 4), "epoch changes reject");
    }

    #[test]
    fn shading_and_fog_match_the_documented_model() {
        let world = scene(-4..4, None);
        let v = ReflectionVolume::pack(&world, 0, [-6; 3], [12; 3], &table(), 193).unwrap();
        let sample = ReflectionSample {
            hit: true,
            cell: [0, 0, 0],
            normal: [0, 1, 0],
            distance: 4.,
            material: TARGET,
        };
        let sun = Sun {
            direction_to_sun: [0., 1., 0.],
            intensity: 1.,
        };
        let shaded = shade_sample(&v, &sample, sun).unwrap();
        let expected = Vec3::new(0.9, 0.05, 0.02) * (0.28 + 0.12 + 1.0);
        assert!((Vec3::from_array(shaded) - expected).length() < 1e-6);
        assert_eq!(fog_mix(BACKGROUND, 12.), BACKGROUND);
        let fogged = fog_mix([1., 1., 1.], 1000.);
        assert!(fogged
            .iter()
            .zip(BACKGROUND)
            .all(|(a, b)| (a - b).abs() < 1e-3));
        // A downward-facing hit receives ambient only from the sun term.
        let down = shade_sample(
            &v,
            &ReflectionSample {
                normal: [0, -1, 0],
                ..sample
            },
            sun,
        )
        .unwrap();
        assert!((Vec3::from_array(down) - Vec3::new(0.9, 0.05, 0.02) * 0.28).length() < 1e-6);
    }
}
