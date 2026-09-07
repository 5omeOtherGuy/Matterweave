//! Authoritative voxel data, reference queries and derived surface geometry.

mod mesh;
mod persistence;
mod ray;

pub use mesh::{Mesh, Vertex};
pub use ray::{RayHit, MAX_RAY_DISTANCE};
use std::collections::BTreeMap;

pub const CHUNK_EDGE: i32 = 16;
pub const CHUNK_VOLUME: usize = 4096;
pub const FORMAT_VERSION: u32 = 1;
pub const GENERATOR_VERSION: u32 = 1;

#[derive(Clone)]
struct Chunk {
    voxels: Box<[u8; CHUNK_VOLUME]>,
    solid: usize,
}

/// Mutable authoritative material data. Material zero is air; all other IDs are solid.
/// Chunk keys use Euclidean division so negative coordinates match positive boundaries.
#[derive(Clone)]
pub struct World {
    seed: u64,
    revision: u64,
    chunks: BTreeMap<[i32; 3], Chunk>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldStats {
    pub chunks: usize,
    pub solid_voxels: usize,
    /// Allocated voxel payload only. Excludes map/allocator overhead and derived meshes.
    pub allocated_bytes: usize,
}

fn address(cell: [i32; 3]) -> ([i32; 3], usize) {
    let chunk = cell.map(|v| v.div_euclid(CHUNK_EDGE));
    let [x, y, z] = cell.map(|v| v.rem_euclid(CHUNK_EDGE) as usize);
    (chunk, x + 16 * (y + 16 * z))
}

impl World {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            revision: 0,
            chunks: BTreeMap::new(),
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn get(&self, cell: [i32; 3]) -> u8 {
        let (key, index) = address(cell);
        self.chunks.get(&key).map_or(0, |chunk| chunk.voxels[index])
    }

    /// Returns whether material data changed. Empty chunks are reclaimed immediately.
    /// At revision exhaustion, edits are rejected without changing data or wrapping.
    pub fn set(&mut self, cell: [i32; 3], material: u8) -> bool {
        if self.get(cell) == material {
            return false;
        }
        // A revision must never wrap and accidentally validate an ancient derived job.
        let Some(next_revision) = self.revision.checked_add(1) else {
            return false;
        };
        let (key, index) = address(cell);
        let chunk = self.chunks.entry(key).or_insert_with(|| Chunk {
            voxels: Box::new([0; CHUNK_VOLUME]),
            solid: 0,
        });
        if chunk.voxels[index] == 0 {
            chunk.solid += 1;
        }
        if material == 0 {
            chunk.solid -= 1;
        }
        chunk.voxels[index] = material;
        if chunk.solid == 0 {
            self.chunks.remove(&key);
        }
        self.revision = next_revision;
        true
    }

    pub fn stats(&self) -> WorldStats {
        WorldStats {
            chunks: self.chunks.len(),
            solid_voxels: self.chunks.values().map(|chunk| chunk.solid).sum(),
            allocated_bytes: self.chunks.len() * CHUNK_VOLUME,
        }
    }

    /// Original deterministic island fixture. Integer generation avoids platform-dependent noise.
    /// This is generator version 1, with terrain, groves, a stone arch and mineral outcrops.
    pub fn generate(seed: u64) -> Self {
        let mut world = Self::new(seed);
        for x in -32..32 {
            for z in -32..32 {
                let height = terrain_height(seed, x, z);
                for y in -8..=height {
                    let material = if y == height {
                        if height <= 0 {
                            4
                        } else {
                            1
                        }
                    } else if y >= height - 2 {
                        2
                    } else {
                        3
                    };
                    world.set([x, y, z], material);
                }
            }
        }
        for (x, z) in [(-18, -15), (-23, 5), (17, -19), (23, 8), (-8, 19), (9, -24)] {
            let base = terrain_height(seed, x, z);
            for y in base + 1..=base + 6 {
                world.set([x, y, z], 5);
            }
            for dx in -3_i32..=3 {
                for dz in -3_i32..=3 {
                    for dy in 3_i32..=8 {
                        if dx.abs() + dz.abs() + (dy - 5).abs() <= 4 {
                            let cell = [x + dx, base + dy, z + dz];
                            if world.get(cell) == 0 {
                                world.set(cell, 6);
                            }
                        }
                    }
                }
            }
        }
        // A six-voxel-wide opening makes an obvious edit/query landmark.
        for x in -9..=-2 {
            for z in -6..=-5 {
                for y in 1..=10 {
                    if x <= -8 || x >= -3 || y >= 9 {
                        world.set([x, y, z], 3);
                    } else {
                        world.set([x, y, z], 0);
                    }
                }
            }
        }
        for (x, z, height) in [(8, 0, 8), (10, -1, 5), (7, 2, 4)] {
            let base = terrain_height(seed, x, z);
            for y in base + 1..=base + height {
                world.set([x, y, z], 7);
            }
        }
        world
    }
}

fn hash(seed: u64, x: i32, z: i32) -> u64 {
    let mut value = seed
        ^ (x as u64).wrapping_mul(0x9e3779b97f4a7c15)
        ^ (z as u64).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn terrain_height(seed: u64, x: i32, z: i32) -> i32 {
    let cx = x.div_euclid(8);
    let cz = z.div_euclid(8);
    let fx = x.rem_euclid(8);
    let fz = z.rem_euclid(8);
    let noise = |dx, dz| (hash(seed, cx + dx, cz + dz) % 7) as i32;
    let a = noise(0, 0) * (8 - fx) + noise(1, 0) * fx;
    let b = noise(0, 1) * (8 - fx) + noise(1, 1) * fx;
    (a * (8 - fz) + b * fz) / 64 - (x.abs().max(z.abs()) - 20).max(0) / 3
}
