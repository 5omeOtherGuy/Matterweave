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
//!
//! Mesh-only geometry (detail volumes and moving mesh-only objects) is not World
//! data, so a volume that must cover it takes a [`MeshProxy`]: a bounded,
//! conservative solid-cell stand-in built from the mesh pool and world transforms
//! the renderer already receives. The authoritative `World` stays authoritative —
//! [`scene_material`] and [`trace_scene`] only add proxy cells where the world is
//! air — and every consumer of a proxy uses those two functions, so the indirect
//! and reflection representations cannot drift apart.
use crate::static_scene::StaticInstance;
use crate::Sun;
use glam::Vec3;
use matterweave_core::{Mesh, RayHit, World};

pub const MAX_FACE_SLOTS: usize = 24_576;
pub const MAX_UPDATE_RAYS: usize = 16_384;
pub const MAX_UPDATE_WORK: usize = 16_384;
/// Largest coverage box a mesh proxy may occupy: the same 64³ cell cap as the
/// reflection source volume.
pub const MAX_MESH_PROXY_CELLS: usize = 64 * 64 * 64;
/// Longest single axis of a mesh proxy coverage box.
pub const MAX_MESH_PROXY_AXIS: u32 = 128;
/// Upper bound on triangle/cell overlap tests one mesh proxy build may perform.
pub const MAX_MESH_PROXY_TESTS: usize = 4 * 1024 * 1024;
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
    /// Attached mesh proxy identity; `None` means unit-voxel geometry only.
    mesh: Option<u64>,
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

/// Mesh-only geometry in the form the renderer already receives it: a resident
/// prototype pool, world placements, and one material per prototype.
///
/// Detail geometry arrives exactly like this from `StaticInstance` records that
/// index a resident mesh pool. A dynamic (CPU-transformed, world-space) mesh-only
/// object is expressed as a one-entry pool with one identity or translated
/// placement, so no second input shape is needed.
///
/// `materials[prototype]` indexes the volume palette and the reflection
/// `MaterialTable`; material zero is air and is rejected.
#[derive(Clone, Copy)]
pub struct MeshGeometry<'a> {
    pub meshes: &'a [Mesh],
    pub instances: &'a [StaticInstance],
    pub materials: &'a [u8],
}

/// Conservative solid-cell proxy for mesh-only geometry, clipped to a coverage box.
///
/// The proxy exists so detail volumes and moving mesh-only objects can take part in
/// the bounded indirect and reflection representations. Both are cell grids indexed
/// by `x + dims.x * (y + dims.y * z)` and traversed by the authoritative `World`
/// DDA, so a proxy is simply those cells: every triangle is transformed to world
/// space by its placement and the cells its surface passes through are marked
/// solid with the prototype material. That is exact for an axis-aligned face on
/// the world cell grid and conservative otherwise: a slanted triangle fills its
/// AABB, and any placement marks at most one extra cell layer, so the proxy can
/// be up to one cell thicker than the source along an axis. Never a miss. Where
/// several triangles mark one cell, the last material written in instance order
/// wins.
///
/// World data stays authoritative: [`scene_material`] reports the world's material
/// wherever the world is solid, so attaching a proxy can never change unit-voxel
/// behaviour. Geometry outside the coverage box does not participate; the caller
/// chooses the box, normally the receiving volume's own footprint.
pub struct MeshProxy {
    origin: [i32; 3],
    dimensions: [u32; 3],
    /// Occupied cells ascending by local index, `(x + dx * (y + dy * z), material)`.
    cells: Vec<(u32, u8)>,
    /// The same cells as a `World`, so traversal reuses the authoritative DDA.
    world: World,
    digest: u64,
}

/// Material of `cell` in the union of the authoritative `World` and a mesh proxy:
/// the world wins wherever it is already solid.
pub(crate) fn scene_material(world: &World, mesh: Option<&MeshProxy>, cell: [i32; 3]) -> u8 {
    match world.get(cell) {
        0 => mesh.map_or(0, |proxy| proxy.material_at(cell)),
        material => material,
    }
}

/// Nearest hit along one segment of the same union. The proxy is consulted only
/// within its coverage box, and an exact tie keeps the authoritative world.
pub(crate) fn trace_scene(
    world: &World,
    mesh: Option<&MeshProxy>,
    origin: [f32; 3],
    direction: [f32; 3],
    max_distance: f32,
) -> Option<RayHit> {
    let hit = world.raycast(origin, direction, max_distance);
    let Some(proxy) = mesh else {
        return hit;
    };
    let limit = hit.as_ref().map_or(max_distance, |world| world.distance);
    let mesh = proxy.raycast(origin, direction, limit);
    if mesh.as_ref().is_some_and(|mesh| {
        hit.as_ref()
            .is_none_or(|world| mesh.distance < world.distance)
    }) {
        mesh
    } else {
        hit
    }
}

/// FNV-1a over the authoritative material of every cell in the footprint, in the
/// same x-fastest order as the packed grids. Independent of the packing code so it
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

// Must match `static_scene::rotate_xz` and the quarter LUT in world.wgsl/
// shadow.wgsl: x' = c*x + s*z, z' = -s*x + c*z with (c,s) from the quarter table.
// `yaw_quarters` is validated to 0..=3 by the caller.
fn rotate_xz(point: [f32; 3], yaw_quarters: u8) -> [f32; 3] {
    match yaw_quarters {
        0 => point,
        1 => [point[2], point[1], -point[0]],
        2 => [-point[0], point[1], -point[2]],
        3 => [-point[2], point[1], point[0]],
        _ => [f32::NAN; 3],
    }
}

impl MeshProxy {
    /// Validates the coverage box, the placements and every transformed triangle
    /// before marking cells. Geometry outside the box is clipped, never an error.
    /// The digest is the [`footprint_digest`] of the marked cells in this box, so a
    /// moved or edited mesh object changes the proxy identity.
    pub fn build(
        geometry: &MeshGeometry<'_>,
        origin: [i32; 3],
        dimensions: [u32; 3],
    ) -> Result<Self, String> {
        if dimensions
            .iter()
            .any(|&d| d == 0 || d > MAX_MESH_PROXY_AXIS)
        {
            return Err(format!(
                "Mesh proxy dimensions must be in 1..={MAX_MESH_PROXY_AXIS}"
            ));
        }
        let slots = dimensions
            .iter()
            .try_fold(1usize, |n, &d| n.checked_mul(d as usize))
            .filter(|&n| n <= MAX_MESH_PROXY_CELLS)
            .ok_or("Mesh proxy exceeds the 64^3 cell cap")?;
        for axis in 0..3 {
            let end = i64::from(origin[axis]) + i64::from(dimensions[axis]);
            if origin[axis] < -8192 || end > 8192 {
                return Err("Mesh proxy bounds exceed the precise voxel coordinate range".into());
            }
        }
        if geometry.materials.len() != geometry.meshes.len() {
            return Err("Mesh proxy needs exactly one material per prototype".into());
        }
        if geometry.materials.contains(&0) {
            return Err("Mesh proxy material zero is air; use a nonzero palette id".into());
        }
        let mut occupancy = Vec::new();
        occupancy
            .try_reserve_exact(slots)
            .map_err(|e| format!("Mesh proxy allocation: {e}"))?;
        occupancy.resize(slots, 0u8);
        let mut tests = 0usize;
        for instance in geometry.instances {
            let mesh = geometry.meshes.get(instance.prototype).ok_or_else(|| {
                format!(
                    "Mesh proxy instance references prototype {} of {}",
                    instance.prototype,
                    geometry.meshes.len()
                )
            })?;
            let material = geometry.materials[instance.prototype];
            if !instance.translation.iter().all(|value| value.is_finite()) {
                return Err("Mesh proxy instance translation must be finite".into());
            }
            if instance.yaw_quarters > 3 {
                return Err("Mesh proxy yaw_quarters must be in 0..=3".into());
            }
            if mesh.indices.len() % 3 != 0 {
                return Err("Mesh proxy indices must be complete triangles".into());
            }
            for triangle in mesh.indices.chunks_exact(3) {
                let mut points = [[0f32; 3]; 3];
                let mut lower = [f32::INFINITY; 3];
                let mut upper = [f32::NEG_INFINITY; 3];
                for (corner, &index) in triangle.iter().enumerate() {
                    let vertex = mesh
                        .vertices
                        .get(index as usize)
                        .ok_or("Mesh proxy index is out of range")?;
                    if !vertex.position.iter().all(|value| value.is_finite()) {
                        return Err("Mesh proxy vertex positions must be finite".into());
                    }
                    let rotated = rotate_xz(vertex.position, instance.yaw_quarters);
                    for axis in 0..3 {
                        let world = rotated[axis] + instance.translation[axis];
                        points[corner][axis] = world;
                        lower[axis] = lower[axis].min(world);
                        upper[axis] = upper[axis].max(world);
                    }
                }
                // Candidate cells for this triangle: its closed world AABB range.
                // That is a superset, so every cell the surface passes through is
                // tested, and the degenerate-axis rule below picks the exact cell
                // world.wgsl's mirror lookup uses (`floor(p - n * eps)`) when the
                // surface lies in a plane, including the cell it faces into when
                // that plane is an integer boundary.
                let triangle = Triangle::new(&points);
                let normal = triangle.normal;
                let mut first = [0i32; 3];
                let mut last = [0i32; 3];
                let mut clipped = false;
                for axis in 0..3 {
                    let box_low = i64::from(origin[axis]);
                    let box_high = box_low + i64::from(dimensions[axis]) - 1;
                    let plane = f64::from(lower[axis]).floor();
                    let extent = f64::from(upper[axis]) - f64::from(lower[axis]);
                    let (low, high) = if extent > 0.0 {
                        (plane, (f64::from(upper[axis]).ceil() - 1.0).max(plane))
                    } else if f64::from(lower[axis]) != plane || normal[axis] < 0.0 {
                        (plane, plane)
                    } else if normal[axis] > 0.0 {
                        (plane - 1.0, plane - 1.0)
                    } else {
                        // A fully degenerate triangle faces nowhere: keep both
                        // sides of the plane so neither lookup can miss it.
                        (plane - 1.0, plane)
                    };
                    if high < box_low as f64 || low > box_high as f64 {
                        clipped = true;
                        break;
                    }
                    first[axis] = low.max(box_low as f64) as i32;
                    last[axis] = high.min(box_high as f64) as i32;
                }
                if clipped {
                    continue;
                }
                for z in first[2]..=last[2] {
                    for y in first[1]..=last[1] {
                        for x in first[0]..=last[0] {
                            tests += 1;
                            if tests > MAX_MESH_PROXY_TESTS {
                                return Err(
                                    "Mesh proxy exceeds the triangle/cell test budget".into()
                                );
                            }
                            let cell = [x, y, z];
                            if !triangle.intersects_cell(cell) {
                                continue;
                            }
                            let local = (x - origin[0]) as usize
                                + dimensions[0] as usize
                                    * ((y - origin[1]) as usize
                                        + dimensions[1] as usize * (z - origin[2]) as usize);
                            occupancy[local] = material;
                        }
                    }
                }
            }
        }
        let occupied = occupancy.iter().filter(|&&material| material != 0).count();
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(occupied)
            .map_err(|e| format!("Mesh proxy allocation: {e}"))?;
        for (index, &material) in occupancy.iter().enumerate() {
            if material != 0 {
                cells.push((index as u32, material));
            }
        }
        let mut world = World::new(0);
        for &(index, material) in &cells {
            let cell = local_cell(origin, dimensions, index);
            if !world.set(cell, material) {
                return Err("Mesh proxy cells could not be stored".into());
            }
        }
        Ok(Self {
            origin,
            dimensions,
            cells,
            digest: footprint_digest(&world, origin, dimensions),
            world,
        })
    }
    pub fn origin(&self) -> [i32; 3] {
        self.origin
    }
    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }
    pub fn occupied_cells(&self) -> usize {
        self.cells.len()
    }
    /// Identity of the marked cells in this coverage box; part of every consumer's
    /// cache key, so a moved or edited mesh object invalidates cached output.
    ///
    /// It is the [`footprint_digest`] of the rasterised cells, not of the source
    /// mesh: it detects any change that moves a surface across a cell boundary and
    /// every edit that adds or removes a cell, but a sub-cell translation that keeps
    /// every triangle inside the cells it already occupied leaves it unchanged. A
    /// caller using this identity to decide whether to rebuild will skip such a
    /// move; that is inherent to a one-cell-resolution proxy and is not a defect,
    /// because the representation is identical there and cached radiance for it
    /// stays valid.
    pub fn digest(&self) -> u64 {
        self.digest
    }
    /// Occupied cells in ascending local-index order.
    pub fn cells(&self) -> impl Iterator<Item = ([i32; 3], u8)> + '_ {
        self.cells
            .iter()
            .map(|&(index, material)| (local_cell(self.origin, self.dimensions, index), material))
    }
    /// Material stored in the proxy, or zero outside the coverage box.
    pub fn material_at(&self, cell: [i32; 3]) -> u8 {
        self.world.get(cell)
    }
    /// Traversal reuses the authoritative `World` DDA over the proxy's own cells.
    /// A segment that cannot enter the coverage box returns without walking it.
    pub fn raycast(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
    ) -> Option<RayHit> {
        let exit = self.segment_exit(origin, direction, max_distance)?;
        self.world.raycast(origin, direction, exit)
    }
    /// Writes every proxy cell that is air in `world`, leaving authoritative
    /// material untouched. Returns the number of cells written. Used to reuse the
    /// existing `World`-only packing path (for example `RayVolume::pack`).
    pub fn merge_into(&self, world: &mut World) -> Result<usize, String> {
        let mut written = 0;
        for (cell, material) in self.cells() {
            if world.get(cell) == 0 {
                if !world.set(cell, material) {
                    return Err(
                        "Mesh proxy cells cannot be merged into this World (streaming \
                         residency or revision exhaustion)"
                            .into(),
                    );
                }
                written += 1;
            }
        }
        Ok(written)
    }
    /// Logical payload: the canonical cell list, the traversal grid and the header.
    pub fn resident_bytes(&self) -> usize {
        self.cells.len() * std::mem::size_of::<(u32, u8)>()
            + self.world.stats().allocated_bytes
            + std::mem::size_of::<Self>()
    }
    /// Exit parameter of the segment inside the closed coverage box, capped by
    /// `max_distance`; `None` when the segment cannot enter it.
    fn segment_exit(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
    ) -> Option<f32> {
        if !origin
            .iter()
            .chain(direction.iter())
            .all(|value| value.is_finite())
            || !max_distance.is_finite()
            || max_distance < 0.0
        {
            return None;
        }
        let norm = direction
            .iter()
            .map(|&value| f64::from(value).powi(2))
            .sum::<f64>()
            .sqrt();
        if norm == 0.0 {
            return None;
        }
        let direction = direction.map(|value| f64::from(value) / norm);
        let origin = origin.map(f64::from);
        let lower = self.origin.map(f64::from);
        let upper: [f64; 3] =
            std::array::from_fn(|axis| lower[axis] + f64::from(self.dimensions[axis]));
        let mut enter = 0f64;
        let mut exit = f64::from(max_distance);
        for axis in 0..3 {
            if direction[axis] == 0.0 {
                if origin[axis] < lower[axis] || origin[axis] > upper[axis] {
                    return None;
                }
                continue;
            }
            let near = (lower[axis] - origin[axis]) / direction[axis];
            let far = (upper[axis] - origin[axis]) / direction[axis];
            let (near, far) = if near <= far {
                (near, far)
            } else {
                (far, near)
            };
            enter = enter.max(near);
            exit = exit.min(far);
            if enter > exit {
                return None;
            }
        }
        Some(exit as f32)
    }
}

/// One triangle of a placed mesh in world space, in the form the cell test needs:
/// computed once per triangle, then reused for every candidate cell.
pub(crate) struct Triangle {
    origin: Vec3,
    edges: [Vec3; 3],
    normal: Vec3,
}

impl Triangle {
    pub(crate) fn new(points: &[[f32; 3]; 3]) -> Self {
        let origin = Vec3::from_array(points[0]);
        let first = Vec3::from_array(points[1]) - origin;
        let second = Vec3::from_array(points[2]) - origin;
        Self {
            origin,
            edges: [first, second, second - first],
            normal: first.cross(second),
        }
    }

    /// Separating-axis test between this triangle and one closed unit cell: the
    /// classic 13 axes (three cell axes, the triangle normal, and the nine
    /// cell-axis × triangle-edge cross products). Any axis that separates the two
    /// convex hulls proves they do not intersect, so a slanted triangle marks only
    /// the cells it really passes through instead of its whole bounding box.
    ///
    /// Contacts count as intersections, which keeps the proxy conservative: a cell
    /// the surface merely grazes is still marked, never a miss. A degenerate
    /// (zero-area) triangle reduces to a segment or point and the same axes still
    /// separate it, with a zero radius on the axes it has collapsed away.
    pub(crate) fn intersects_cell(&self, cell: [i32; 3]) -> bool {
        let half = Vec3::splat(0.5);
        let center = Vec3::new(
            cell[0] as f32 + 0.5,
            cell[1] as f32 + 0.5,
            cell[2] as f32 + 0.5,
        );
        let corner = self.origin - center;
        let corners = [corner, corner + self.edges[0], corner + self.edges[1]];
        for axis in 0..3 {
            let (min, max) = min_max([corners[0][axis], corners[1][axis], corners[2][axis]]);
            if min > half[axis] || max < -half[axis] {
                return false;
            }
        }
        let distance = self.normal.dot(corner);
        if distance.abs() > half.dot(self.normal.abs()) {
            return false;
        }
        for edge in self.edges {
            for box_axis in 0..3 {
                // Cell axis × triangle edge; signs are irrelevant because both
                // ends of the projected interval are compared below.
                let axis = match box_axis {
                    0 => Vec3::new(0., -edge.z, edge.y),
                    1 => Vec3::new(edge.z, 0., -edge.x),
                    _ => Vec3::new(-edge.y, edge.x, 0.),
                };
                let (min, max) = min_max([
                    axis.dot(corners[0]),
                    axis.dot(corners[1]),
                    axis.dot(corners[2]),
                ]);
                let radius = half.dot(axis.abs());
                if min > radius || max < -radius {
                    return false;
                }
            }
        }
        true
    }
}

fn min_max(values: [f32; 3]) -> (f32, f32) {
    (
        values[0].min(values[1]).min(values[2]),
        values[0].max(values[1]).max(values[2]),
    )
}

fn local_cell(origin: [i32; 3], dimensions: [u32; 3], index: u32) -> [i32; 3] {
    let dx = dimensions[0];
    let dy = dimensions[1];
    let x = index % dx;
    let y = (index / dx) % dy;
    let z = index / (dx * dy);
    [
        origin[0] + x as i32,
        origin[1] + y as i32,
        origin[2] + z as i32,
    ]
}

/// Fixed-size residency with fallible allocation after validation. Coordinates
/// are limited to +/-8192 so the face bias remains representable in f32.
/// Palette entries are linear diffuse reflectances, immutable and in [0,1].
/// Face order is +X,-X,+Y,-Y,+Z,-Z. Unit voxel geometry is sampled from `World`;
/// mesh-only geometry participates through an attached [`MeshProxy`], which cannot
/// change a cell the world already occupies.
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
    mesh: Option<MeshProxy>,
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
            mesh: None,
        })
    }
    /// Attach or clear the mesh-only geometry that takes part in this volume: the
    /// proxy is sampled as additional solid cells and is a trace target, while
    /// authoritative `World` cells stay authoritative. The representation is part
    /// of the cache identity, so publishing one clears the current output
    /// immediately and the next `update` recomputes from the first face.
    ///
    /// A proxy is a bounded stand-in, not a proof of coverage: the caller decides
    /// when the volume covers the scene's mesh-only geometry.
    pub fn set_mesh_proxy(&mut self, mesh: Option<MeshProxy>) {
        self.mesh = mesh;
        self.clear();
    }
    /// Identity of the attached proxy, or `None` for unit-voxel geometry only.
    /// Compare it with the current scene before reusing a volume across mesh
    /// placement or mesh edits.
    pub fn mesh_digest(&self) -> Option<u64> {
        self.mesh.as_ref().map(MeshProxy::digest)
    }
    /// Whether mesh-only geometry is represented by this volume.
    pub fn has_mesh_proxy(&self) -> bool {
        self.mesh.is_some()
    }
    fn clear(&mut self) {
        self.values.fill([0.; 4]);
        self.cursor = 0;
        self.sample_index = 0;
        self.sum = Vec3::ZERO;
        self.key = None;
    }
    pub fn resident_bytes(&self) -> usize {
        self.values.len() * 16
            + std::mem::size_of::<Self>()
            + self.mesh.as_ref().map_or(0, |proxy| proxy.resident_bytes())
    }
    /// Whether the volume holds a complete, publishable result for its current
    /// key. A fresh volume, a cleared volume and a partially updated volume are
    /// all incomplete; only a finished key is publishable. This is the CPU
    /// work-unit gate: it bounds preparation work, not Android frames or
    /// presentation latency (see the D3.2 log).
    pub fn complete(&self) -> bool {
        self.key.is_some() && self.cursor == self.values.len()
    }
    /// Upper bound on the remaining `work` units needed to finish the current
    /// key: at most one face inspection per face slot per sample. A changed key
    /// starts with at most `values.len() * samples` pending; each bounded
    /// `update` call that makes progress strictly reduces it. This bounds CPU
    /// preparation slices, not wall-clock time or frame presentation.
    pub fn pending_work(&self) -> usize {
        if self.complete() {
            return 0;
        }
        self.values
            .len()
            .saturating_sub(self.cursor)
            .saturating_mul(self.samples as usize)
            .saturating_sub(self.sample_index as usize)
    }
    pub fn valid_for(&self, world: &World, epoch: u64, sun: Sun) -> bool {
        self.complete()
            && light_key(sun).is_ok_and(|sun| {
                self.key
                    == Some(Key {
                        epoch,
                        revision: world.revision(),
                        sun,
                        mesh: self.mesh_digest(),
                    })
            })
    }
    pub(crate) fn cached_sun(&self) -> Option<[f32; 4]> {
        self.key.map(|k| k.sun)
    }
    /// Source identity for publication: the caller's replacement epoch and the
    /// authoritative revision, plus completion. An incomplete volume is never
    /// publishable, so a superseded or partially recomputed key cannot be
    /// uploaded. Mesh-only coverage is not re-checked here; it is
    /// the caller's contract, as documented on [`IndirectVolume::set_mesh_proxy`].
    pub(crate) fn source_valid(&self, world: &World, epoch: u64) -> bool {
        self.complete()
            && self
                .key
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
            mesh: self.mesh_digest(),
        });
        if key != self.key || key.is_none() {
            self.clear();
            self.key = key;
        }
        let light = light?;
        let direction = Vec3::new(light[0], light[1], light[2]);
        let mesh = self.mesh.as_ref();
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
            if scene_material(world, mesh, cell) == 0 || scene_material(world, mesh, neighbor) != 0
            {
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
            if let Some(hit) = trace_scene(
                world,
                mesh,
                origin.to_array(),
                ray.to_array(),
                self.distance,
            ) {
                let hn = Vec3::from_array(hit.normal.map(|n| n as f32));
                let cosine = hn.dot(direction).max(0.);
                if cosine > 0. {
                    // Place the shadow origin outside the exact entry face.
                    let point = origin + ray * hit.distance + hn * EPSILON;
                    stats.rays += 1;
                    if trace_scene(
                        world,
                        mesh,
                        point.to_array(),
                        direction.to_array(),
                        self.distance,
                    )
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
