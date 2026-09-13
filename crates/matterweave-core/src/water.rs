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
//! Documented limitation of this version: the sea level is a constant, so digging
//! a hole below sea level on dry land does not create a lake and draining terrain
//! under a lake does not lower the surface. Adding per-column water levels is a
//! later change, not an accidental behaviour.

use crate::landscape::SEA_LEVEL;
use crate::material;
use crate::{Mesh, Vertex, World, CHUNK_EDGE, CHUNK_VOLUME};

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
        let mut any = false;
        for lx in 0..CHUNK_EDGE {
            for lz in 0..CHUNK_EDGE {
                let x = origin[0] + lx;
                let z = origin[1] + lz;
                if !self.column_is_flooded(x, z, min_y, max_y) {
                    continue;
                }
                flooded[lx as usize][lz as usize] = true;
                any = true;
            }
        }
        if !any {
            return mesh;
        }
        // Greedy merge runs along +x, then stack equal runs along +z. This is not
        // a full maximal-rectangle search: it is bounded, deterministic and
        // already collapses an open sea into a handful of quads per chunk.
        let color = material::color(material::WATER);
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
                    && flooded[(column + width) as usize][row as usize]
                {
                    width += 1;
                }
                // Equal runs stack in +z.
                let mut height = 1;
                while row + height < CHUNK_EDGE
                    && (0..width).all(|dx| flooded[(column + dx) as usize][(row + height) as usize])
                {
                    height += 1;
                }
                let x0 = origin[0] + column;
                let z0 = origin[1] + row;
                let x1 = x0 + width;
                let z1 = z0 + height;
                let base = mesh.vertices.len() as u32;
                // Top face winding follows the core mesher: cross(U, V) points +Y.
                for position in [
                    [x0 as f32, SEA_LEVEL as f32, z1 as f32],
                    [x1 as f32, SEA_LEVEL as f32, z1 as f32],
                    [x1 as f32, SEA_LEVEL as f32, z0 as f32],
                    [x0 as f32, SEA_LEVEL as f32, z0 as f32],
                ] {
                    mesh.vertices.push(Vertex {
                        position,
                        normal,
                        color,
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

    /// Whether one column holds water at the sea level plane.
    ///
    /// The scan starts from the top of the column only when an edit could have
    /// placed solid material there. For an unedited landscape column the
    /// generator's own surface is the top, so an open-water column costs a few
    /// reads instead of one per metre of empty sky above it. Edits are detected
    /// per column stack through the streaming overrides, which is the same
    /// authority `World::get` reads.
    fn column_is_flooded(&self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool {
        let top = self.column_scan_start(x, z, min_y, max_y);
        let mut y = top;
        while y >= SEA_LEVEL {
            if material::is_solid(self.get([x, y, z])) {
                return false;
            }
            y -= 1;
        }
        let mut y = SEA_LEVEL - 1;
        while y >= min_y {
            if material::is_solid(self.get([x, y, z])) {
                return true;
            }
            y -= 1;
        }
        false
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

/// Bytes of one water mesh payload, for bounded residency accounting.
pub fn water_mesh_bytes(mesh: &Mesh) -> usize {
    mesh.vertices.capacity() * std::mem::size_of::<Vertex>() + mesh.indices.capacity() * 4
}

const _: () = assert!(CHUNK_VOLUME == (CHUNK_EDGE * CHUNK_EDGE * CHUNK_EDGE) as usize);
