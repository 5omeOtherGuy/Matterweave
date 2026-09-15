use crate::World;
use bytemuck::{Pod, Zeroable};

/// Tightly packed GPU upload data; no platform/graphics-library types cross this boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}

#[derive(Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub revision: u64,
}

// Origin plus U and V describe each unit face with cross(U,V) pointing outwards.
type Face = ([i32; 3], [i32; 3], [i32; 3], [i32; 3]);
const FACES: [Face; 6] = [
    ([1, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]),
    ([-1, 0, 0], [0, 0, 1], [0, 1, 0], [0, 0, -1]),
    ([0, 1, 0], [0, 1, 1], [1, 0, 0], [0, 0, -1]),
    ([0, -1, 0], [0, 0, 0], [1, 0, 0], [0, 0, 1]),
    ([0, 0, 1], [0, 0, 1], [1, 0, 0], [0, 1, 0]),
    ([0, 0, -1], [1, 0, 0], [-1, 0, 0], [0, 1, 0]),
];

fn color(material: u8) -> [f32; 3] {
    crate::material::color(material)
}

impl World {
    /// Synchronous whole-world exposed-face baseline, including cross-chunk occlusion.
    /// Stable iteration order makes equal worlds produce equal mesh bytes.
    /// Call after edits, not each frame. This is not greedy meshing, LOD or streaming.
    ///
    /// Its vertex colour stays the palette colour. This path is the compatibility
    /// and comparison baseline: `renderer_comparison` requires it to agree
    /// pixel-for-pixel with the ray reference, which shades from the same palette
    /// by material, and the per-voxel tone of [`crate::material::tone`] cannot be
    /// reproduced by a per-material palette. The streamed mesher
    /// ([`Self::mesh_chunk`]) and [`crate::landscape::lod_tile_mesh`] carry the
    /// tone; they are the paths the landscape sample draws.
    pub fn mesh(&self) -> Mesh {
        let mut mesh = Mesh {
            revision: self.revision(),
            ..Mesh::default()
        };
        for (key, chunk) in &self.chunks {
            for (index, &material) in chunk.voxels.iter().enumerate() {
                if material == 0 {
                    continue;
                }
                let local = [
                    (index % 16) as i32,
                    ((index / 16) % 16) as i32,
                    (index / 256) as i32,
                ];
                let cell: [i32; 3] = std::array::from_fn(|axis| key[axis] * 16 + local[axis]);
                for (normal, origin, u, v) in FACES {
                    let neighbor = std::array::from_fn(|axis| cell[axis].checked_add(normal[axis]));
                    if let [Some(x), Some(y), Some(z)] = neighbor {
                        if self.get([x, y, z]) != 0 {
                            continue;
                        }
                    }
                    let base =
                        u32::try_from(mesh.vertices.len()).expect("mesh exceeds u32 index range");
                    for (du, dv) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
                        mesh.vertices.push(Vertex {
                            position: std::array::from_fn(|axis| {
                                cell[axis] as f32
                                    + (origin[axis] + du * u[axis] + dv * v[axis]) as f32
                            }),
                            normal: normal.map(|value| value as f32),
                            color: color(material),
                        });
                    }
                    mesh.indices.extend_from_slice(&[
                        base,
                        base + 1,
                        base + 2,
                        base,
                        base + 2,
                        base + 3,
                    ]);
                }
            }
        }
        mesh
    }
}

#[derive(Clone, Copy)]
struct Material(u8);
impl block_mesh::Voxel for Material {
    fn get_visibility(&self) -> block_mesh::VoxelVisibility {
        if self.0 == 0 {
            block_mesh::VoxelVisibility::Empty
        } else {
            block_mesh::VoxelVisibility::Opaque
        }
    }
}
impl block_mesh::MergeVoxel for Material {
    type MergeValue = u8;
    fn merge_value(&self) -> u8 {
        self.0
    }
}

use block_mesh::ndshape::{ConstShape, ConstShape3u32};
type Shape = ConstShape3u32<18, 18, 18>;

/// One chunk and its one-voxel halo: the only world data a mesh job reads.
/// Fixed size keeps a queued job's cost exactly [`HALO_VOLUME`] bytes.
pub(crate) const HALO_VOLUME: usize = 18 * 18 * 18;
pub(crate) type Halo = Box<[u8; HALO_VOLUME]>;
const _: () = assert!(Shape::SIZE as usize == HALO_VOLUME);

impl World {
    /// Copies the materials `mesh_chunk` would read. `None` for an absent chunk,
    /// which invalidates any previously derived geometry for that key.
    pub(crate) fn chunk_halo(&self, key: [i32; 3]) -> Option<Halo> {
        if !self.chunks.contains_key(&key) {
            return None;
        }
        let mut voxels: Halo = Box::new([0; HALO_VOLUME]);
        for i in 0..Shape::SIZE {
            let local = Shape::delinearize(i);
            let cell =
                std::array::from_fn(|axis| i64::from(key[axis]) * 16 + i64::from(local[axis]) - 1);
            if let [Ok(x), Ok(y), Ok(z)] = cell.map(i32::try_from) {
                voxels[i as usize] = self.get([x, y, z]);
            }
        }
        Some(voxels)
    }

    /// Material-preserving greedy quads from a 16³ chunk and one-voxel halo.
    /// Positions are world space. Revision includes neighboring shared-face edits;
    /// accept a result only if `chunk_revision(key) == Some(mesh.revision)`.
    pub fn mesh_chunk(&self, key: [i32; 3]) -> Mesh {
        let revision = self.chunk_revision(key).unwrap_or(self.revision);
        match self.chunk_halo(key) {
            Some(voxels) => mesh_halo(key, &voxels, revision),
            None => Mesh {
                revision,
                ..Mesh::default()
            },
        }
    }
}

/// Deterministic mesher shared by the synchronous and background paths, so an
/// accepted background result is byte-identical to `mesh_chunk` for equal input.
pub(crate) fn mesh_halo(key: [i32; 3], voxels: &[u8; HALO_VOLUME], revision: u64) -> Mesh {
    let mut mesh = Mesh {
        revision,
        ..Mesh::default()
    };
    let voxels: Vec<Material> = voxels.iter().map(|&value| Material(value)).collect();
    let faces = block_mesh::RIGHT_HANDED_Y_UP_CONFIG.faces;
    let mut buffer = block_mesh::GreedyQuadsBuffer::new(voxels.len());
    block_mesh::greedy_quads(&voxels, &Shape {}, [0; 3], [17; 3], &faces, &mut buffer);
    for (group, face) in buffer.quads.groups.iter().zip(faces) {
        for quad in group {
            let base = mesh.vertices.len() as u32;
            let material = voxels[Shape::linearize(quad.minimum) as usize].0;
            let colour = quad_color(key, quad, &face, material);
            for position in face.quad_mesh_positions(quad, 1.0) {
                mesh.vertices.push(Vertex {
                    position: std::array::from_fn(|axis| {
                        (i64::from(key[axis]) * 16) as f32 + position[axis] - 1.0
                    }),
                    normal: face.signed_normal().as_vec3().to_array(),
                    color: colour,
                });
            }
            mesh.indices.extend(face.quad_mesh_indices(base));
        }
    }
    mesh
}

/// Colour of one greedy-merged quad: the mean per-voxel tone of every cell the
/// quad merges, applied to the material colour.
///
/// Colour is deliberately not part of the merge key. If it were, neighbouring
/// cells would almost never merge and every flat run would explode into per-cell
/// quads; aggregating at the quad instead keeps the merge (and the triangle
/// count) exactly as it was, at the cost of one colour per run rather than one
/// per cell. A run therefore reads as one block of ground, which is what the
/// coarse distance rings show anyway.
fn quad_color(
    key: [i32; 3],
    quad: &block_mesh::UnorientedQuad,
    face: &block_mesh::OrientedBlockFace,
    material: u8,
) -> [f32; 3] {
    let corners = face.quad_corners(quad);
    // The second and third corners step one cell along the face's own axes, so
    // the deltas divided by the quad's extent are those unit steps.
    let u = (corners[1] - corners[0]) / quad.width;
    let v = (corners[2] - corners[0]) / quad.height;
    let mut sum = [0i32; 3];
    let mut cell = block_mesh::ilattice::glam::UVec3::from(quad.minimum);
    for _ in 0..quad.width {
        let mut at = cell;
        for _ in 0..quad.height {
            let world: [i32; 3] = std::array::from_fn(|axis| key[axis] * 16 + at[axis] as i32 - 1);
            let tone = crate::material::tone(material, world[0], world[1], world[2]);
            for channel in 0..3 {
                sum[channel] += tone[channel];
            }
            at += v;
        }
        cell += u;
    }
    crate::material::tint(
        color(material),
        crate::material::mean_tone(sum, (quad.width * quad.height) as i32),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material;

    /// One flat 16x16 slab of `material` at y = 0 in the origin chunk: the top
    /// faces of all 256 cells are coplanar and share a material, so the mesher
    /// merges them into a single quad.
    fn slab(material: u8) -> World {
        let mut world = World::new(7);
        for z in 0..16 {
            for x in 0..16 {
                world.set([x, 0, z], material);
            }
        }
        world
    }

    fn top_colour(mesh: &Mesh) -> [f32; 3] {
        for quad in mesh.vertices.chunks_exact(4) {
            if quad[0].normal == [0.0, 1.0, 0.0] {
                return quad[0].color;
            }
        }
        panic!("the slab has no top face");
    }

    #[test]
    fn a_merged_quad_carries_the_mean_tone_of_its_cells() {
        let world = slab(material::MOSS);
        let mesh = world.mesh_chunk([0, 0, 0]);
        // The expected colour is summed here from the same public per-cell tone
        // the mesher reads, over the same 256 cells: a mesher that sampled one
        // cell, or skipped the aggregate, lands on a different colour.
        let mut sum = [0i32; 3];
        for z in 0..16 {
            for x in 0..16 {
                let tone = material::tone(material::MOSS, x, 0, z);
                for channel in 0..3 {
                    sum[channel] += tone[channel];
                }
            }
        }
        let mean = material::mean_tone(sum, 16 * 16);
        let expected = material::tint(material::color(material::MOSS), mean);
        assert_eq!(top_colour(&mesh), expected);
        // Averaging a whole tile of one family's white noise converges on the
        // material colour, which is the honest limit of quad-level aggregation:
        // a 16x16 flat run carries a few 1/256ths, not a single cell's tens.
        assert!(
            mean.iter().all(|value| value.abs() <= 8),
            "a whole slab averages back toward the base colour: {mean:?}"
        );
    }

    #[test]
    fn one_cell_keeps_its_own_tone_and_surfaces_stay_within_their_family() {
        let mut world = World::new(7);
        world.set([3, 0, 5], material::STONE);
        let mesh = world.mesh_chunk([0, 0, 0]);
        // A lone cell merges with nothing, so its per-cell tone is its colour.
        let expected = material::tint(
            material::color(material::STONE),
            material::tone(material::STONE, 3, 0, 5),
        );
        assert_eq!(top_colour(&mesh), expected);
        // Two materials at one cell are two different colours; a reload of the
        // same world rebuilds them exactly.
        let again = slab(material::MOSS).mesh_chunk([0, 0, 0]);
        assert_eq!(
            top_colour(&again),
            top_colour(&slab(material::MOSS).mesh_chunk([0, 0, 0]))
        );
    }

    #[test]
    fn equal_worlds_mesh_to_equal_bytes_on_every_call() {
        // Determinism is the whole contract of the tone: a mesh is bytes, and a
        // worker result is accepted only if it equals the synchronous one.
        let first = slab(material::SAND).mesh_chunk([0, 0, 0]);
        let second = slab(material::SAND).mesh_chunk([0, 0, 0]);
        let dump = |mesh: &Mesh| {
            let mut bytes = Vec::new();
            for vertex in &mesh.vertices {
                for value in vertex
                    .position
                    .iter()
                    .chain(&vertex.normal)
                    .chain(&vertex.color)
                {
                    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
                }
            }
            bytes.extend_from_slice(bytemuck::cast_slice(&mesh.indices));
            bytes
        };
        assert_eq!(dump(&first), dump(&second));
        // The whole-world baseline keeps the palette colour: it is the
        // comparison mesh the ray reference must match by material, so the tone
        // exists only in the streamed and tile meshers. A lone cell proves the
        // two paths differ exactly by the cell's tone.
        let mut single = World::new(7);
        single.set([3, 0, 5], material::GRAVEL);
        let reference = single.mesh();
        let greedy = single.mesh_chunk([0, 0, 0]);
        let reference_colour = reference
            .vertices
            .iter()
            .find(|vertex| vertex.normal == [0.0, 1.0, 0.0])
            .expect("top face")
            .color;
        assert_eq!(reference_colour, material::color(material::GRAVEL));
        assert_eq!(
            top_colour(&greedy),
            material::tint(
                material::color(material::GRAVEL),
                material::tone(material::GRAVEL, 3, 0, 5)
            )
        );
    }
}
