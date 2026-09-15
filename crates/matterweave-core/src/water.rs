//! Derived water surface meshes.
//!
//! Water is never stored in the authoritative world: a column whose topmost solid
//! cell lies below [`SEA_LEVEL`] is flooded, and this module derives the visible
//! surface for it. Deriving rather than storing keeps collision, saves, meshing
//! and edits on one representation of terrain — a lake costs no voxels and no
//! water ever has to be reconciled with an edit.
//!
//! The surface is a set of top faces at the plane `y = SEA_LEVEL`, greedy-merged
//! over runs of flooded columns. A chunk that does not contain sea level has no
//! water mesh at all, and the derivation is coupled to
//! [`World::chunk_revision`], so an edit invalidates water exactly when it
//! invalidates the terrain mesh for the same chunk.
//!
//! # The vertex colour carries depth, not colour
//!
//! A water vertex stores the depth of the bed under its own corner in
//! `color[0]`, as `metres / WATER_MAX_DEPTH_M` clamped to one, with `color[1]`
//! and `color[2]` zero. The renderer draws this mesh through its own fragment
//! entry, which owns the water palette, so the surface colour never has to be a
//! vertex attribute - and the depth is the one quantity the shader cannot derive
//! for itself, because the bed under the surface is not in the fragment's reach.
//! The depth is integer metres from the authoritative world, divided by a power
//! of two, so the encoding is exact and identical on every machine.
//!
//! Quads are capped at [`WATER_QUAD_MAX_M`] metres a side for the same reason:
//! the four corners are the only depth samples a quad has, and a merged run the
//! width of a chunk would interpolate a shoal into a trench.
//!
//! Documented limitation of this version: the sea level is a constant, so digging
//! a hole below sea level on dry land does not create a lake and draining terrain
//! under a lake does not lower the surface. Adding per-column water levels is a
//! later change, not an accidental behaviour.

use crate::landscape::SEA_LEVEL;

use crate::material;
use crate::{Mesh, Vertex, World, CHUNK_EDGE, CHUNK_VOLUME};

/// Depth in metres that encodes as a full `color[0]` of one. Deeper beds clamp;
/// the shader's tint has long saturated by then. A power of two, so the division
/// is exact.
pub const WATER_MAX_DEPTH_M: f32 = 32.0;

/// Longest side of one derived water quad, in metres. The cap bounds how far a
/// corner depth is interpolated, at a cost of at most `(CHUNK_EDGE / 4)^2` quads
/// per chunk for an open-sea chunk that would otherwise merge into one.
pub const WATER_QUAD_MAX_M: i32 = 4;

/// The chunk layer that contains the sea surface. Water meshes exist only here:
/// every other layer would draw the same surface again.
pub fn sea_level_chunk_y() -> i32 {
    SEA_LEVEL.div_euclid(CHUNK_EDGE)
}

impl World {
    /// Derived water surface of one chunk, or an empty mesh.
    ///
    /// A column contributes a quad when no solid cell exists at or above
    /// [`SEA_LEVEL`] in the simulation band and at least one solid cell exists
    /// below it. The revision is the chunk's terrain revision, so a caller may
    /// cache water meshes with exactly the invalidation rules it already uses for
    /// [`World::mesh_chunk`].
    pub fn water_mesh_chunk(&self, key: [i32; 3]) -> Mesh {
        let revision = self.chunk_revision(key).unwrap_or(self.revision);
        let mut mesh = Mesh {
            revision,
            ..Mesh::default()
        };
        if key[1] != sea_level_chunk_y() {
            return mesh;
        }
        let (min_y, max_y) = self.stream_y_range();
        let origin = [key[0] * CHUNK_EDGE, key[2] * CHUNK_EDGE];
        let mut flooded = [[false; CHUNK_EDGE as usize]; CHUNK_EDGE as usize];
        // Depth of the bed under each flooded column, in whole metres.
        let mut depth = [[0i32; CHUNK_EDGE as usize]; CHUNK_EDGE as usize];
        let mut any = false;
        for lx in 0..CHUNK_EDGE {
            for lz in 0..CHUNK_EDGE {
                let x = origin[0] + lx;
                let z = origin[1] + lz;
                let Some(bed) = self.column_bed(x, z, min_y, max_y) else {
                    continue;
                };
                flooded[lx as usize][lz as usize] = true;
                depth[lx as usize][lz as usize] = (SEA_LEVEL - bed).max(0);
                any = true;
            }
        }
        if !any {
            return mesh;
        }
        // Greedy merge runs along +x, then stack equal runs along +z. This is not
        // a full maximal-rectangle search: it is bounded, deterministic and
        // already collapses an open sea into a few quads per chunk, up to the
        // depth-interpolation cap.
        let normal = [0.0, 1.0, 0.0];
        let mut row = 0;
        while row < CHUNK_EDGE {
            let mut column = 0;
            while column < CHUNK_EDGE {
                if !flooded[column as usize][row as usize] {
                    column += 1;
                    continue;
                }
                let mut width = 1;
                while column + width < CHUNK_EDGE
                    && width < WATER_QUAD_MAX_M
                    && flooded[(column + width) as usize][row as usize]
                {
                    width += 1;
                }
                // Equal runs stack in +z.
                let mut height = 1;
                while row + height < CHUNK_EDGE
                    && height < WATER_QUAD_MAX_M
                    && (0..width).all(|dx| flooded[(column + dx) as usize][(row + height) as usize])
                {
                    height += 1;
                }
                let x0 = origin[0] + column;
                let z0 = origin[1] + row;
                let x1 = x0 + width;
                let z1 = z0 + height;
                let base = mesh.vertices.len() as u32;
                // A corner of the quad samples the flooded column it touches,
                // which is the last one inside the run on each axis.
                let (near_x, far_x) = (column as usize, (column + width - 1) as usize);
                let (near_z, far_z) = (row as usize, (row + height - 1) as usize);
                // Top face winding follows the core mesher: cross(U, V) points +Y.
                for (position, corner) in [
                    ([x0 as f32, SEA_LEVEL as f32, z1 as f32], (near_x, far_z)),
                    ([x1 as f32, SEA_LEVEL as f32, z1 as f32], (far_x, far_z)),
                    ([x1 as f32, SEA_LEVEL as f32, z0 as f32], (far_x, near_z)),
                    ([x0 as f32, SEA_LEVEL as f32, z0 as f32], (near_x, near_z)),
                ] {
                    mesh.vertices.push(Vertex {
                        position,
                        normal,
                        color: encode_depth(depth[corner.0][corner.1]),
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
                for dz in 0..height {
                    for dx in 0..width {
                        flooded[(column + dx) as usize][(row + dz) as usize] = false;
                    }
                }
                column += width;
            }
            row += 1;
        }
        mesh
    }

    /// Height of the bed under one flooded column, or `None` when the column
    /// holds no water at the sea level plane.
    ///
    /// The scan starts from the top of the column only when an edit could have
    /// placed solid material there. For an unedited landscape column the
    /// generator's own surface is the top, so an open-water column costs a few
    /// reads instead of one per metre of empty sky above it. Edits are detected
    /// per column stack through the streaming overrides, which is the same
    /// authority `World::get` reads. The downward scan that proves the column is
    /// wet already stops on the bed, so the depth is free.
    fn column_bed(&self, x: i32, z: i32, min_y: i32, max_y: i32) -> Option<i32> {
        let top = self.column_scan_start(x, z, min_y, max_y);
        let mut y = top;
        while y >= SEA_LEVEL {
            if material::is_solid(self.get([x, y, z])) {
                return None;
            }
            y -= 1;
        }
        let mut y = SEA_LEVEL - 1;
        while y >= min_y {
            if material::is_solid(self.get([x, y, z])) {
                return Some(y);
            }
            y -= 1;
        }
        None
    }

    /// Highest cell a flooded-column scan has to inspect.
    ///
    /// A generated column cannot be higher than the generator says unless it was
    /// edited, so an unedited column starts one metre above its generated
    /// surface (clamped to the sea level when that is already lower than sea
    /// level, which skips the above-sea scan entirely).
    fn column_scan_start(&self, x: i32, z: i32, min_y: i32, max_y: i32) -> i32 {
        let ceiling = max_y - 1;
        if self.terrain != crate::TerrainSource::Landscape {
            return ceiling;
        }
        let edited = self.streaming.as_ref().is_some_and(|stream| {
            let cell = [x, 0, z];
            let key = crate::address(cell).0;
            self.stream_y_chunks()
                .any(|chunk_y| stream.overrides.contains_key(&[key[0], chunk_y, key[2]]))
        });
        if edited {
            return ceiling;
        }
        (crate::landscape::height_at(self.seed, x, z) + 1).clamp(min_y, ceiling)
    }
}

/// Depth in whole metres as a water vertex carries it. See the module note: the
/// water vertex colour is a depth channel, not a colour.
pub fn encode_depth(metres: i32) -> [f32; 3] {
    [
        (metres.max(0) as f32 / WATER_MAX_DEPTH_M).min(1.0),
        0.0,
        0.0,
    ]
}

/// Bytes of one water mesh payload, for bounded residency accounting.
pub fn water_mesh_bytes(mesh: &Mesh) -> usize {
    mesh.vertices.capacity() * std::mem::size_of::<Vertex>() + mesh.indices.capacity() * 4
}

const _: () = assert!(CHUNK_VOLUME == (CHUNK_EDGE * CHUNK_EDGE * CHUNK_EDGE) as usize);
