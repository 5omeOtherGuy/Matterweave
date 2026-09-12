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
//! A fragment's mirror strength is the mirror value of the material stored for the
//! cell at `floor(p - n * SURFACE_OFFSET)`. The authoritative voxel material is read
//! from the source volume; a cell that the World leaves as air takes the attached
//! [`MeshProxy`]'s material instead, which is how a detail volume or moving
//! mesh-only object becomes reflective and traceable.
//!
//! Bound: at most one pending publication, no background job, one host-visible
//! coherent destination buffer per resource, and at most
//! `MAX_REFLECTION_CELLS * 4 + REFLECTION_PALETTE_BYTES` bytes uploaded.
use crate::indirect::{scene_material, trace_scene, MeshProxy};
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
///
/// Mesh-only geometry is baked in at pack time through [`MeshProxy`]; the
/// authoritative provenance (epoch, revision, seed) is stored here rather than
/// taken from the composed pack, whose source world is a derived copy.
#[derive(Debug)]
pub struct ReflectionVolume {
    pack: RayVolume,
    palette: [[f32; 4]; 256],
    trace_steps: u32,
    digest: u64,
    source_epoch: u64,
    source_revision: u64,
    source_seed: u64,
    mesh_digest: Option<u64>,
}

impl ReflectionVolume {
    /// Validates before allocation. Each axis is in `1..=64` and the product is at
    /// most `64³`; endpoints must lie within `+/-8192`. Unit-voxel geometry only:
    /// use [`ReflectionVolume::pack_with_mesh`] to include mesh-only geometry.
    pub fn pack(
        world: &World,
        epoch: u64,
        origin: [i32; 3],
        dimensions: [u32; 3],
        materials: &MaterialTable,
        trace_steps: u32,
    ) -> Result<Self, String> {
        Self::build(
            world,
            epoch,
            origin,
            dimensions,
            materials,
            trace_steps,
            None,
        )
    }
    /// Same source contract as [`ReflectionVolume::pack`], plus mesh-only geometry:
    /// every proxy cell the authoritative `World` leaves as air is written into a
    /// derived copy of the world before packing, so the grid the shader reads (and
    /// the packed palette) covers the detail volume or moving mesh-only object.
    /// Where the world is already solid the world's material wins, which is the
    /// same union rule the indirect volume and [`crate::reflection::reflect_sample_with_mesh`]
    /// apply.
    ///
    /// The proxy is part of the packed content, not of `valid_for`: reuse a volume
    /// across a mesh placement change only after comparing
    /// [`ReflectionVolume::source_mesh_digest`], or use
    /// [`ReflectionVolume::valid_for_scene`].
    pub fn pack_with_mesh(
        world: &World,
        epoch: u64,
        origin: [i32; 3],
        dimensions: [u32; 3],
        materials: &MaterialTable,
        trace_steps: u32,
        mesh: &MeshProxy,
    ) -> Result<Self, String> {
        Self::build(
            world,
            epoch,
            origin,
            dimensions,
            materials,
            trace_steps,
            Some(mesh),
        )
    }
    fn build(
        world: &World,
        epoch: u64,
        origin: [i32; 3],
        dimensions: [u32; 3],
        materials: &MaterialTable,
        trace_steps: u32,
        mesh: Option<&MeshProxy>,
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
        // The pack is reused unchanged: it owns the grid layout, cell cap and
        // palette packing, and only its source object differs when a proxy is
        // merged in. Its stored provenance is the derived copy's, so validity is
        // checked against the authoritative fields kept below instead.
        let merged;
        let source = match mesh {
            Some(mesh) => {
                let mut copy = world.clone();
                mesh.merge_into(&mut copy)?;
                merged = copy;
                &merged
            }
            None => world,
        };
        let pack = RayVolume::pack(source, epoch, origin, dimensions, materials.colors())?;
        Ok(Self {
            pack,
            palette: materials.palette(),
            trace_steps,
            digest: footprint_digest(world, origin, dimensions),
            source_epoch: epoch,
            source_revision: world.revision(),
            source_seed: world.seed(),
            mesh_digest: mesh.map(MeshProxy::digest),
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
        self.source_epoch
    }
    pub fn source_revision(&self) -> u64 {
        self.source_revision
    }
    pub fn source_seed(&self) -> u64 {
        self.source_seed
    }
    /// Identity of the mesh-only geometry baked into this pack, or `None` for a
    /// unit-voxel-only pack. Reuse a volume across a mesh placement or mesh edit
    /// only while this still equals the current proxy's digest.
    pub fn source_mesh_digest(&self) -> Option<u64> {
        self.mesh_digest
    }
    /// Conservative: even edits outside the crop invalidate this pack, and the
    /// footprint digest is recomputed so an equal-revision replacement cannot be
    /// accepted. Cost is one `World::get` per cell (at most 262,144), incurred only
    /// at publication, never per frame. Mesh-only geometry is not re-checked here;
    /// see [`ReflectionVolume::valid_for_scene`].
    pub fn valid_for(&self, world: &World, epoch: u64) -> bool {
        self.source_epoch == epoch
            && self.source_revision == world.revision()
            && self.source_seed == world.seed()
            && self.digest == footprint_digest(world, self.origin(), self.dimensions())
    }
    /// [`ReflectionVolume::valid_for`] plus the identity of the mesh-only geometry
    /// this pack was built from, so a caller can decide whether a volume still
    /// represents the current scene without rebuilding it.
    pub fn valid_for_scene(&self, world: &World, epoch: u64, mesh_digest: Option<u64>) -> bool {
        self.mesh_digest == mesh_digest && self.valid_for(world, epoch)
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
        let material_bytes = std::mem::size_of_val(self.materials());
        ReflectionMemoryStats {
            material_bytes,
            palette_bytes: REFLECTION_PALETTE_BYTES,
            upload_bytes: material_bytes + REFLECTION_PALETTE_BYTES,
            resident_bytes: material_bytes + REFLECTION_PALETTE_BYTES + size_of::<Self>(),
        }
    }
}

/// FNV-1a over the authoritative material of every cell in the footprint, in the
/// same x-fastest order as the packed grids. Independent of the packing code so it
/// can detect a replaced scene whose revision and seed are unchanged. Owned by
/// [`crate::indirect`] because the mesh proxy uses the same function for its own
/// identity; the public path is preserved here.
pub use crate::indirect::footprint_digest;

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
///
/// Unit-voxel scenes only. A volume packed with [`ReflectionVolume::pack_with_mesh`]
/// must be probed with [`reflect_sample_with_mesh`], otherwise the oracle cannot see
/// the mesh-only cells the shader's grid contains.
pub fn reflect_sample(
    volume: &ReflectionVolume,
    world: &World,
    point: [f32; 3],
    normal: [f32; 3],
    eye: [f32; 3],
) -> ReflectionSample {
    reflect_in_scene(volume, world, None, point, normal, eye)
}

/// [`reflect_sample`] over the same union the packed grid is built from: the
/// authoritative `World` plus mesh-only geometry already merged into `volume`.
/// The oracle stays independent of the shader traversal: it clips to the volume,
/// then traces the authoritative DDA and the proxy's own DDA.
pub fn reflect_sample_with_mesh(
    volume: &ReflectionVolume,
    world: &World,
    mesh: &MeshProxy,
    point: [f32; 3],
    normal: [f32; 3],
    eye: [f32; 3],
) -> ReflectionSample {
    reflect_in_scene(volume, world, Some(mesh), point, normal, eye)
}

fn reflect_in_scene(
    volume: &ReflectionVolume,
    world: &World,
    mesh: Option<&MeshProxy>,
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
    // The is_finite guard short-circuits NaN, so `exit <= 0.` is exact here.
    if !exit.is_finite() || exit <= 0. {
        return ReflectionSample::miss(0.);
    }
    let start = std::array::from_fn(|a| origin[a].floor() as i32);
    // Self-intersection: a solid start cell would report a zero-distance hit on
    // the surface being shaded.
    if scene_material(world, mesh, start) != 0 {
        return ReflectionSample::miss(exit);
    }
    let Some(hit) = trace_scene(world, mesh, origin.to_array(), direction.to_array(), exit) else {
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
    use crate::indirect::{MeshGeometry, MeshProxy};
    use crate::static_scene::StaticInstance;
    use glam::Vec3;
    use matterweave_core::Mesh;

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

    // =======================================================================
    // D3.1 mesh-only reflection probe. Declared criterion:
    //
    // C7 a mesh-only surface becomes reflective: the packed mirror strength at
    //    its cell is zero before (air) and the assigned material's strength
    //    afterwards, and the very grid and palette the shader reads
    //    (group-0 bindings 4 and 5) carry that material.
    // C8 the mesh-only object participates in the traced geometry, in both
    //    directions: a reflection off an existing world mirror now hits the
    //    mesh-only object instead of the far wall, and a reflection off the
    //    mesh-only surface itself lands on world geometry.
    // =======================================================================
    const OBJECT: u8 = 4;
    const OBJECT_CELL: [i32; 3] = [0, 1, 0];
    const MIRROR_CELL: [i32; 3] = [-2, -1, 0];
    const MESH_ORIGIN: [i32; 3] = [-4, -2, -4];
    const MESH_DIMS: [u32; 3] = [8, 7, 8];

    fn mesh_table() -> MaterialTable {
        let mut color = [[0.5; 3]; 256];
        color[0] = [0.; 3];
        color[MIRROR as usize] = [0.42, 0.77, 0.72];
        color[WALL as usize] = [0.65; 3];
        color[TARGET as usize] = [0.9, 0.05, 0.02];
        color[OBJECT as usize] = [0.05, 0.8, 0.1];
        let mut table = MaterialTable::new(color).unwrap();
        table.set_mirror(MIRROR, 1.0).unwrap();
        table.set_mirror(OBJECT, 1.0).unwrap();
        table
    }

    /// Unit-voxel floor plus one world mirror cell and a tall wall the traced
    /// ray reaches when nothing stands in its way.
    fn mesh_world() -> World {
        let mut world = World::new(9);
        for x in MESH_ORIGIN[0]..MESH_ORIGIN[0] + MESH_DIMS[0] as i32 {
            for z in MESH_ORIGIN[2]..MESH_ORIGIN[2] + MESH_DIMS[2] as i32 {
                world.set([x, -1, z], TARGET);
            }
        }
        world.set(MIRROR_CELL, MIRROR);
        for y in 0..5 {
            for z in MESH_ORIGIN[2]..MESH_ORIGIN[2] + MESH_DIMS[2] as i32 {
                world.set([3, y, z], WALL);
            }
        }
        world
    }

    /// One unit cube: the engine's own mesher output for a single voxel.
    fn mesh_prototype() -> Mesh {
        let mut voxel = World::new(0);
        voxel.set([0, 0, 0], 1);
        voxel.mesh()
    }

    fn mesh_object() -> StaticInstance {
        StaticInstance {
            prototype: 0,
            translation: [0., 1., 0.],
            yaw_quarters: 0,
        }
    }

    fn mesh_proxy(instances: &[StaticInstance]) -> MeshProxy {
        let meshes = [mesh_prototype()];
        let materials = [OBJECT];
        MeshProxy::build(
            &MeshGeometry {
                meshes: &meshes,
                instances,
                materials: &materials,
            },
            MESH_ORIGIN,
            MESH_DIMS,
        )
        .unwrap()
    }

    fn mesh_local(cell: [i32; 3]) -> usize {
        let local: [usize; 3] =
            std::array::from_fn(|axis| (cell[axis] - MESH_ORIGIN[axis]) as usize);
        local[0] + MESH_DIMS[0] as usize * (local[1] + MESH_DIMS[1] as usize * local[2])
    }

    #[test]
    fn c7_mesh_only_surface_becomes_reflective() {
        let world = mesh_world();
        let table = mesh_table();
        let plain = ReflectionVolume::pack(
            &world,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
        )
        .unwrap();
        assert_eq!(plain.material_at(OBJECT_CELL), 0, "before: air");
        assert_eq!(plain.mirror_at(OBJECT_CELL), 0.0, "before: nonreflective");
        let proxy = mesh_proxy(&[mesh_object()]);
        let volume = ReflectionVolume::pack_with_mesh(
            &world,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
            &proxy,
        )
        .unwrap();
        assert_eq!(volume.material_at(OBJECT_CELL), OBJECT);
        assert_eq!(volume.mirror_at(OBJECT_CELL), 1.0);
        // The exact buffers the shader reads: one u32 per cell (binding 4) and
        // 256 vec4s with the mirror strength in w (binding 5).
        assert_eq!(
            volume.materials()[mesh_local(OBJECT_CELL)],
            u32::from(OBJECT)
        );
        assert_eq!(volume.palette()[OBJECT as usize][3], 1.0);
        // World data stays authoritative where both are solid.
        assert_eq!(volume.material_at([3, 1, 1]), WALL);
        assert_eq!(volume.mirror_at([3, 1, 1]), 0.0);
        // Provenance stays the authority's, even though packing used a derived
        // copy of the world; the mesh identity travels next to it.
        assert_eq!(volume.source_revision(), world.revision());
        assert_eq!(volume.source_seed(), world.seed());
        assert_eq!(volume.source_mesh_digest(), Some(proxy.digest()));
        assert!(volume.valid_for(&world, 0));
        assert!(volume.valid_for_scene(&world, 0, Some(proxy.digest())));
        assert!(!volume.valid_for_scene(&world, 0, None));
        assert!(
            !volume.valid_for_scene(&world, 0, plain.source_mesh_digest()),
            "a unit-voxel-only pack is a different scene"
        );
    }

    #[test]
    fn c8_mesh_only_geometry_participates_in_reflection_traces() {
        let world = mesh_world();
        let table = mesh_table();
        let plain = ReflectionVolume::pack(
            &world,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
        )
        .unwrap();
        let proxy = mesh_proxy(&[mesh_object()]);
        let volume = ReflectionVolume::pack_with_mesh(
            &world,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
            &proxy,
        )
        .unwrap();
        // A world mirror cell: its top face reflects up and away from the eye.
        let point = [-1.5, 0.0, 0.5];
        let normal = [0., 1., 0.];
        let eye = [-3.0, 1.5, 0.5];
        let before = reflect_sample(&plain, &world, point, normal, eye);
        assert!(
            before.hit && before.cell[0] == 3 && before.material == WALL,
            "before: the ray passes through the object's cell to the wall: {before:?}"
        );
        let after = reflect_sample_with_mesh(&volume, &world, &proxy, point, normal, eye);
        assert!(after.hit, "after: the mesh-only object is a trace target");
        assert_eq!(after.cell, OBJECT_CELL);
        assert_eq!(after.material, OBJECT);
        assert!(
            after.distance < before.distance,
            "the object is nearer than the wall: {} vs {}",
            after.distance,
            before.distance
        );
        // The mesh-only surface reflects onto world geometry in the other
        // direction, so it is a mirror rather than only a receiver.
        let top = [0.5, 2.0, 0.5];
        let up = [0., 1., 0.];
        let above = [-1.0, 3.0, 0.5];
        assert_eq!(
            plain.mirror_at(surface_cell(top, up)),
            0.0,
            "before the slice this fragment was nonreflective, so the shader skipped it"
        );
        assert_eq!(volume.mirror_at(surface_cell(top, up)), 1.0);
        let mirrored = reflect_sample_with_mesh(&volume, &world, &proxy, top, up, above);
        assert!(
            mirrored.hit && mirrored.material == WALL && mirrored.cell[0] == 3,
            "the mesh mirror reflects onto the wall: {mirrored:?}"
        );
    }

    #[test]
    fn g1_fractional_mesh_surfaces_still_start_inside_their_own_cell() {
        // Production detail geometry is placed at fractional metres, so its
        // surface planes are strictly inside a proxy cell: the mirror lookup
        // finds the material, but the reflected ray's first cell is the
        // surface's own cell and both the shipped shader (`iteration == 0u`)
        // and this oracle terminate it. This test pins that behaviour so the
        // gap cannot be reported as working; see the D3.1 log for the proposed
        // world.wgsl + oracle diff that lifts it.
        let world = mesh_world();
        let table = mesh_table();
        let fractional = StaticInstance {
            prototype: 0,
            translation: [0.25, 0.9, 0.0],
            yaw_quarters: 0,
        };
        let proxy = mesh_proxy(&[fractional]);
        let volume = ReflectionVolume::pack_with_mesh(
            &world,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
            &proxy,
        )
        .unwrap();
        let top = [0.75, 1.9, 0.5];
        let up = [0., 1., 0.];
        let cell = surface_cell(top, up);
        assert_eq!(cell, [0, 1, 0], "the surface is inside a marked cell");
        assert_eq!(
            volume.mirror_at(cell),
            1.0,
            "the packed mirror strength is the mesh material's"
        );
        assert_eq!(
            volume.material_at(cell),
            OBJECT,
            "the shipped grid carries the mesh material at this cell"
        );
        let sample = reflect_sample_with_mesh(&volume, &world, &proxy, top, up, [-1.0, 3.0, 0.5]);
        assert!(
            !sample.hit,
            "the ray starts in the surface's own cell: {sample:?}"
        );
        assert!(sample.distance > 0., "a miss still reports its exit");
        // The same geometry shifted onto a cell boundary does reflect.
        let aligned = mesh_proxy(&[mesh_object()]);
        let aligned_volume = ReflectionVolume::pack_with_mesh(
            &world,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
            &aligned,
        )
        .unwrap();
        assert!(
            reflect_sample_with_mesh(
                &aligned_volume,
                &world,
                &aligned,
                [0.5, 2.0, 0.5],
                up,
                [-1.0, 3.0, 0.5]
            )
            .hit,
            "grid-aligned mesh faces reflect onto world geometry"
        );
    }

    // ===================================================================
    // D3.2 dynamic-response probes. Declared criteria, asserted rather than
    // eyeballed:
    //
    // R1 moving reflector: lifting a mesh-only object redirects a reflection
    //    that hit it (onto the far wall, at a longer distance) while a
    //    reflection travelling away from the move and the packed cells of a
    //    second, static mesh-only object stay bit-identical; the two packs
    //    carry different proxy identities and each is valid only for its own.
    // R2 edit and supersession: removing one world wall cell invalidates the
    //    previous pack (valid_for and valid_for_scene both refuse it, including
    //    after an A->B->A round trip at a new revision), the repacked volume
    //    reports the edited reflectivity (a former wall hit is now a miss)
    //    while an untouched footprint digest and the mesh-proxy identity are
    //    unchanged.
    // ===================================================================
    const LIFTED_CELL: [i32; 3] = [0, 3, 0];
    const CONTROL_CELL: [i32; 3] = [-3, 0, -3];

    fn lifted_object() -> StaticInstance {
        StaticInstance {
            prototype: 0,
            translation: [0., 3., 0.],
            yaw_quarters: 0,
        }
    }

    fn control_object() -> StaticInstance {
        StaticInstance {
            prototype: 0,
            translation: [-3., 0., -3.],
            yaw_quarters: 0,
        }
    }

    #[test]
    fn moving_reflector_redirects_response_and_static_control_holds() {
        let world = mesh_world();
        let table = mesh_table();
        let rest = mesh_proxy(&[mesh_object(), control_object()]);
        let lifted = mesh_proxy(&[lifted_object(), control_object()]);
        assert_ne!(
            rest.digest(),
            lifted.digest(),
            "a moved reflector must change the pack identity"
        );
        assert_eq!(rest.occupied_cells(), 2);
        assert_eq!(lifted.occupied_cells(), 2);
        let at_rest = ReflectionVolume::pack_with_mesh(
            &world,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
            &rest,
        )
        .unwrap();
        let raised = ReflectionVolume::pack_with_mesh(
            &world,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
            &lifted,
        )
        .unwrap();
        // The static control object packs identically in both volumes, and the
        // moved cell is occupied in exactly one pack each way.
        assert_eq!(at_rest.material_at(CONTROL_CELL), OBJECT);
        assert_eq!(raised.material_at(CONTROL_CELL), OBJECT);
        assert_eq!(at_rest.material_at(LIFTED_CELL), 0);
        assert_eq!(raised.material_at(LIFTED_CELL), OBJECT);
        assert_eq!(raised.material_at(OBJECT_CELL), 0);
        assert_eq!(at_rest.mirror_at(CONTROL_CELL), 1.0);
        assert_eq!(raised.mirror_at(CONTROL_CELL), 1.0);
        // The affected ray (c8): off the world mirror it hits the object at
        // rest, and the far wall once the object is lifted out of the path.
        let point = [-1.5, 0.0, 0.5];
        let normal = [0., 1., 0.];
        let eye = [-3.0, 1.5, 0.5];
        let before = reflect_sample_with_mesh(&at_rest, &world, &rest, point, normal, eye);
        assert!(
            before.hit && before.cell == OBJECT_CELL && before.material == OBJECT,
            "at rest the ray hits the reflector: {before:?}"
        );
        let after = reflect_sample_with_mesh(&raised, &world, &lifted, point, normal, eye);
        assert!(
            after.hit && after.cell[0] == 3 && after.material == WALL,
            "lifted, the same ray reaches the wall: {after:?}"
        );
        assert!(
            after.distance > before.distance,
            "the wall is farther than the reflector was: {} vs {}",
            after.distance,
            before.distance
        );
        // The static control: a reflection off the same mirror travelling -X,
        // away from both reflector positions and the wall, is bit-identical.
        let away = [1.0, 1.5, 0.5];
        let control_before = reflect_sample_with_mesh(&at_rest, &world, &rest, point, normal, away);
        let control_after = reflect_sample_with_mesh(&raised, &world, &lifted, point, normal, away);
        assert!(!control_before.hit && !control_after.hit);
        assert_eq!(
            control_before, control_after,
            "a reflection that cannot see the move must not drift"
        );
        // Each pack is valid only for the proxy identity it was built from.
        assert!(at_rest.valid_for(&world, 0));
        assert!(at_rest.valid_for_scene(&world, 0, Some(rest.digest())));
        assert!(!at_rest.valid_for_scene(&world, 0, Some(lifted.digest())));
        assert!(!at_rest.valid_for_scene(&world, 0, None));
        assert!(raised.valid_for_scene(&world, 0, Some(lifted.digest())));
        assert!(!raised.valid_for_scene(&world, 0, Some(rest.digest())));
    }

    #[test]
    fn world_edit_updates_reflectivity_and_superseded_pack_is_refused() {
        let world = mesh_world();
        let table = mesh_table();
        let proxy = mesh_proxy(&[mesh_object()]);
        let before = ReflectionVolume::pack_with_mesh(
            &world,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
            &proxy,
        )
        .unwrap();
        // Sanity: the mesh-top ray hits the wall cell the edit will remove.
        let top = [0.5, 2.0, 0.5];
        let up = [0., 1., 0.];
        let above = [-1.0, 3.0, 0.5];
        let was = reflect_sample_with_mesh(&before, &world, &proxy, top, up, above);
        assert!(
            was.hit && was.cell == [3, 3, 0] && was.material == WALL,
            "the mesh mirror reflects onto the wall cell under test: {was:?}"
        );
        assert_eq!(before.material_at([3, 3, 0]), WALL);
        // Remove the wall cells on the ray's climb: one authoritative edit per
        // cell. The ray enters the x = 3 plane at y ≈ 3.67 ([3, 3, 0]) and
        // climbs into [3, 4, 0], so both must go for the ray to leave the wall.
        let mut edited = world.clone();
        assert!(edited.set([3, 3, 0], 0));
        assert!(edited.set([3, 4, 0], 0));
        // The previous pack is refused for the edited scene by both checks.
        assert!(!before.valid_for(&edited, 0));
        assert!(!before.valid_for_scene(&edited, 0, Some(proxy.digest())));
        // The repacked volume reports the edited reflectivity: the former hit
        // now leaves through the hole, while the mesh cell itself is unchanged.
        let after = ReflectionVolume::pack_with_mesh(
            &edited,
            0,
            MESH_ORIGIN,
            MESH_DIMS,
            &table,
            DEFAULT_TRACE_STEPS,
            &proxy,
        )
        .unwrap();
        assert_eq!(after.material_at([3, 3, 0]), 0);
        assert_eq!(after.material_at([3, 4, 0]), 0);
        assert_eq!(after.material_at(OBJECT_CELL), OBJECT);
        assert_eq!(after.mirror_at(OBJECT_CELL), 1.0);
        let now = reflect_sample_with_mesh(&after, &edited, &proxy, top, up, above);
        assert!(
            !now.hit,
            "the removed wall cell no longer reflects: {now:?}"
        );
        assert!(now.distance > 0., "a miss still reports its exit");
        assert!(after.valid_for(&edited, 0));
        assert!(after.valid_for_scene(&edited, 0, Some(proxy.digest())));
        // Untouched geometry keeps its identity: the footprint outside the wall
        // and the rebuilt proxy digest are stable, while the edited footprint
        // changes. Compared before the round trip below restores the cells.
        assert_eq!(
            footprint_digest(&world, [-4, -2, -4], [6, 7, 8]),
            footprint_digest(&edited, [-4, -2, -4], [6, 7, 8]),
            "x in -4..1 excludes the wall"
        );
        assert_ne!(
            footprint_digest(&world, MESH_ORIGIN, MESH_DIMS),
            footprint_digest(&edited, MESH_ORIGIN, MESH_DIMS)
        );
        assert_eq!(mesh_proxy(&[mesh_object()]).digest(), proxy.digest());
        // Restoring the cells at a new revision does not resurrect the old pack.
        edited.set([3, 3, 0], WALL);
        edited.set([3, 4, 0], WALL);
        assert!(!before.valid_for(&edited, 0));
        assert!(!before.valid_for_scene(&edited, 0, Some(proxy.digest())));
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
