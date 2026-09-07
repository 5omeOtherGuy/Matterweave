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
    match material {
        1 => [0.29, 0.48, 0.27], // Moss
        2 => [0.35, 0.24, 0.17], // Soil
        3 => [0.47, 0.51, 0.52], // Weathered stone
        4 => [0.72, 0.65, 0.46], // Sand
        5 => [0.30, 0.20, 0.13], // Wood
        6 => [0.19, 0.38, 0.28], // Canopy
        7 => [0.42, 0.77, 0.72], // Mineral
        _ => [0.69, 0.46, 0.33],
    }
}

impl World {
    /// Synchronous whole-world exposed-face baseline, including cross-chunk occlusion.
    /// Stable iteration order makes equal worlds produce equal mesh bytes.
    /// Call after edits, not each frame. This is not greedy meshing, LOD or streaming.
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

impl World {
    /// Material-preserving greedy quads from a 16³ chunk and one-voxel halo.
    /// Positions are world space. Revision includes neighboring shared-face edits;
    /// accept a result only if `chunk_revision(key) == Some(mesh.revision)`.
    pub fn mesh_chunk(&self, key: [i32; 3]) -> Mesh {
        use block_mesh::ndshape::{ConstShape, ConstShape3u32};
        type Shape = ConstShape3u32<18, 18, 18>;
        let mut mesh = Mesh {
            revision: self.chunk_revision(key).unwrap_or(self.revision),
            ..Mesh::default()
        };
        if !self.chunks.contains_key(&key) {
            return mesh;
        }
        let mut voxels = vec![Material(0); Shape::SIZE as usize];
        for i in 0..Shape::SIZE {
            let local = Shape::delinearize(i);
            let cell =
                std::array::from_fn(|axis| i64::from(key[axis]) * 16 + i64::from(local[axis]) - 1);
            if let [Ok(x), Ok(y), Ok(z)] = cell.map(i32::try_from) {
                voxels[i as usize] = Material(self.get([x, y, z]));
            }
        }
        let faces = block_mesh::RIGHT_HANDED_Y_UP_CONFIG.faces;
        let mut buffer = block_mesh::GreedyQuadsBuffer::new(voxels.len());
        block_mesh::greedy_quads(&voxels, &Shape {}, [0; 3], [17; 3], &faces, &mut buffer);
        for (group, face) in buffer.quads.groups.iter().zip(faces) {
            for quad in group {
                let base = mesh.vertices.len() as u32;
                let material = voxels[Shape::linearize(quad.minimum) as usize].0;
                for position in face.quad_mesh_positions(quad, 1.0) {
                    mesh.vertices.push(Vertex {
                        position: std::array::from_fn(|axis| {
                            (i64::from(key[axis]) * 16) as f32 + position[axis] - 1.0
                        }),
                        normal: face.signed_normal().as_vec3().to_array(),
                        color: color(material),
                    });
                }
                mesh.indices.extend(face.quad_mesh_indices(base));
            }
        }
        mesh
    }
}
