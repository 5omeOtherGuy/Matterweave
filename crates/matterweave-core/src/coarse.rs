//! Bounded, read-only coarse terrain tiles derived from authoritative [`World`] state.
//!
//! A coarse tile is a derived preview, never authoritative data: it is not stored in a
//! world, is never used for collision and never overrides fine data. Gameplay keeps
//! using the authoritative fine chunks; a coarse tile exists so a later renderer can
//! show terrain at distance from a bounded, deterministic derivation.
//!
//! # Sources
//!
//! Derivation reads an immutable [`World`] and never mutates it:
//!
//! - A **streaming** world uses exactly the sources [`World::stream_around`] publishes
//!   fine chunks from: explicit override chunks (edits and saved chunks, including the
//!   explicit empty override that tombstones a deleted chunk), the
//!   snapshot-authoritative legacy central square where a missing chunk means air, and
//!   procedural generation elsewhere. The classification is per fine chunk, so a
//!   level-2 tile that straddles the legacy boundary derives both halves correctly.
//!   Residency is deliberately ignored: [`World::get`] only sees resident chunks, so
//!   deriving from residency would change output after eviction.
//! - A **non-streaming** world uses only its stored authoritative chunks and never
//!   procedurally generates, because the world itself does not.
//!
//! # Supported requests
//!
//! - Levels 1 and 2 only. A tile always holds [`COARSE_TILE_EDGE`] cubed coarse cells;
//!   at level `level` one coarse cell aggregates `2^level` cubed fine cells and the
//!   tile spans `16 << level` fine cells per axis.
//! - Every covered fine-coordinate box must be representable in `i32`; checked
//!   arithmetic rejects overflow with [`CoarseError::OutOfRange`]. That is the
//!   non-streaming bound policy: any stored authoritative coordinate persistence
//!   accepts is derivable, and no streaming limit is applied to it.
//! - A **streaming** tile must intersect the simulation domain (`x, z` in
//!   `-WORLD_LIMIT..WORLD_LIMIT`, `y` in `STREAM_MIN_Y..STREAM_MAX_Y`). Wholly
//!   disjoint tiles are rejected with [`CoarseError::OutsideSupportedDomain`]. Partially
//!   intersecting tiles are allowed and cells outside the domain are air; procedural
//!   generation is only defined inside the domain.
//! - Work and memory per request are bounded by the level: at most `(16 << level)^3`
//!   fine-cell samples, a 4096-byte tile and, transiently, the covered fine chunks.
//!   Level 2 holds up to 64 fine chunks (about 256 KiB of voxel payload plus their
//!   chunk headers) for the duration of one derivation; level 1 holds up to 8. That
//!   per-call scratch is bounded by the level and key, never extended by input.
//!
//! # Aggregation policy
//!
//! A coarse cell is occupied if **any** covered fine cell is solid. The aggregation is
//! conservative: thin holes and one-cell openings can be filled, and the fine ring
//! stays the authority. The material is the material of the topmost occupied fine
//! cell; when several fine cells share that height, the lowest x, then the lowest z
//! wins. The rule is order independent and therefore deterministic across requests.
//!
//! # Provenance
//!
//! [`CoarseTile::source_revision`] is the whole-world [`World::revision`] at derivation
//! time. Persisted per-chunk revisions do not describe evicted history, so this core
//! has no stable per-tile revision: any edit anywhere may change the value, and equal
//! revisions across worlds prove nothing about identity. A consumer must treat a
//! revision change as "re-derive", never as proof that a tile is still valid.
//! [`CoarseTile::seed`] and [`CoarseTile::generator_version`] identify the procedural
//! source.

use crate::streaming::generated_chunk;
use crate::{
    address, Chunk, World, CHUNK_EDGE, CHUNK_VOLUME, GENERATOR_VERSION, STREAM_MAX_Y, STREAM_MIN_Y,
    WORLD_LIMIT,
};
use std::fmt;

/// Coarse cells per tile axis. Every tile is this cubed, so tile material memory is
/// `16^3 = 4096` bytes at every supported level.
pub const COARSE_TILE_EDGE: i32 = 16;

/// Lowest supported coarse level: one coarse cell aggregates `2^3` fine cells and the
/// tile spans 32 fine cells per axis.
pub const COARSE_MIN_LEVEL: u8 = 1;

/// Highest supported coarse level: one coarse cell aggregates `4^3` fine cells and the
/// tile spans 64 fine cells per axis.
pub const COARSE_MAX_LEVEL: u8 = 2;

/// Fine cells per tile axis for a supported level (`16 << level`), or `None`.
pub fn tile_span(level: u8) -> Option<i32> {
    if (COARSE_MIN_LEVEL..=COARSE_MAX_LEVEL).contains(&level) {
        Some(CHUNK_EDGE << level)
    } else {
        None
    }
}

/// Reasons a coarse tile request is rejected. A valid request for an all-air region is
/// `Ok` with [`CoarseTile::is_empty`], never an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoarseError {
    /// `level` is outside `COARSE_MIN_LEVEL..=COARSE_MAX_LEVEL`.
    UnsupportedLevel { level: u8 },
    /// The key's covered fine-coordinate box is not representable in `i32`.
    OutOfRange { level: u8, key: [i32; 3] },
    /// A streaming world request whose tile does not intersect the simulation domain.
    /// Non-streaming worlds do not return this error.
    OutsideSupportedDomain { level: u8, key: [i32; 3] },
}

impl fmt::Display for CoarseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedLevel { level } => {
                write!(f, "coarse level {level} is not supported")
            }
            Self::OutOfRange { level, key } => {
                write!(f, "coarse level {level} key {key:?} is not representable")
            }
            Self::OutsideSupportedDomain { level, key } => write!(
                f,
                "coarse level {level} key {key:?} is outside the simulation domain"
            ),
        }
    }
}

impl std::error::Error for CoarseError {}

/// A bounded, read-only coarse tile derived from authoritative world material.
///
/// The tile carries its coarse materials plus the identity a consumer needs to place
/// and invalidate it: level, key, fine-cell origin and span, source seed, conservative
/// source revision and generator version. See the module documentation for the source,
/// aggregation and provenance policies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoarseTile {
    level: u8,
    key: [i32; 3],
    origin: [i32; 3],
    span: i32,
    seed: u64,
    source_revision: u64,
    generator_version: u32,
    materials: Vec<u8>,
    solid_cells: usize,
}

impl CoarseTile {
    /// Level this tile was derived at.
    pub fn level(&self) -> u8 {
        self.level
    }

    /// Tile key in coarse tile units; multiply by [`Self::span`] for the world origin.
    pub fn key(&self) -> [i32; 3] {
        self.key
    }

    /// Fine-cell world coordinate of local cell `[0, 0, 0]`.
    pub fn origin(&self) -> [i32; 3] {
        self.origin
    }

    /// Fine cells covered per tile axis (`16 << level`).
    pub fn span(&self) -> i32 {
        self.span
    }

    /// Seed of the world this tile was derived from.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Conservative whole-world provenance at derivation time. See the module
    /// documentation: this is not a stable per-tile revision.
    pub fn source_revision(&self) -> u64 {
        self.source_revision
    }

    /// Version of the procedural terrain source.
    pub fn generator_version(&self) -> u32 {
        self.generator_version
    }

    /// Number of occupied coarse cells.
    pub fn solid_cells(&self) -> usize {
        self.solid_cells
    }

    /// Whether no coarse cell is occupied.
    pub fn is_empty(&self) -> bool {
        self.solid_cells == 0
    }

    /// Coarse materials in local index order `x + 16 * (y + 16 * z)`, air being zero.
    pub fn materials(&self) -> &[u8] {
        &self.materials
    }

    /// Material at a local cell in `0..16` per axis, or `None` when out of range.
    pub fn material(&self, local: [i32; 3]) -> Option<u8> {
        Some(self.materials[coarse_index(local)?])
    }

    /// Fine-cell world coordinate of a local coarse cell in `0..16` per axis, or
    /// `None` when out of range. Each local step covers `1 << level` fine cells.
    pub fn world_cell(&self, local: [i32; 3]) -> Option<[i32; 3]> {
        coarse_index(local)?;
        let aggregation = 1 << self.level;
        Some([
            self.origin[0] + local[0] * aggregation,
            self.origin[1] + local[1] * aggregation,
            self.origin[2] + local[2] * aggregation,
        ])
    }
}

impl World {
    /// Derives the bounded coarse tile for `level` and `key` from this world.
    ///
    /// The derivation is pure and eviction independent; see the module documentation
    /// for the source rules, aggregation policy, provenance and supported domains.
    ///
    /// # Errors
    ///
    /// [`CoarseError::UnsupportedLevel`] for a level outside 1..=2,
    /// [`CoarseError::OutOfRange`] when the covered fine coordinates are not
    /// representable in `i32`, and [`CoarseError::OutsideSupportedDomain`] when a
    /// streaming world request does not intersect the simulation domain.
    pub fn coarse_tile(&self, level: u8, key: [i32; 3]) -> Result<CoarseTile, CoarseError> {
        let Some(span) = tile_span(level) else {
            return Err(CoarseError::UnsupportedLevel { level });
        };
        let Some(origin) = tile_origin(key, span) else {
            return Err(CoarseError::OutOfRange { level, key });
        };
        if self.streaming.is_some() && !intersects_simulation_domain(origin, span) {
            return Err(CoarseError::OutsideSupportedDomain { level, key });
        }
        let aggregation = 1 << level;
        let chunks_per_axis = (span / CHUNK_EDGE) as usize;
        let chunk_base = origin.map(|v| v.div_euclid(CHUNK_EDGE));
        let mut sources = Vec::with_capacity(chunks_per_axis * chunks_per_axis * chunks_per_axis);
        for z in 0..chunks_per_axis {
            for y in 0..chunks_per_axis {
                for x in 0..chunks_per_axis {
                    let chunk_key = [
                        chunk_base[0] + x as i32,
                        chunk_base[1] + y as i32,
                        chunk_base[2] + z as i32,
                    ];
                    sources.push(self.coarse_chunk_source(chunk_key));
                }
            }
        }
        let mut tile = CoarseTile {
            level,
            key,
            origin,
            span,
            seed: self.seed,
            source_revision: self.revision,
            generator_version: GENERATOR_VERSION,
            materials: vec![0; CHUNK_VOLUME],
            solid_cells: 0,
        };
        if !sources.iter().flatten().any(|source| source.solid() != 0) {
            return Ok(tile);
        }
        for cz in 0..COARSE_TILE_EDGE {
            for cy in 0..COARSE_TILE_EDGE {
                for cx in 0..COARSE_TILE_EDGE {
                    let base = [
                        origin[0] + cx * aggregation,
                        origin[1] + cy * aggregation,
                        origin[2] + cz * aggregation,
                    ];
                    let material =
                        topmost_material(&sources, chunks_per_axis, chunk_base, base, aggregation);
                    if material != 0 {
                        let index = (cx + COARSE_TILE_EDGE * (cy + COARSE_TILE_EDGE * cz)) as usize;
                        tile.materials[index] = material;
                        tile.solid_cells += 1;
                    }
                }
            }
        }
        Ok(tile)
    }

    /// Authoritative material source for one fine chunk, or `None` for empty.
    fn coarse_chunk_source(&self, key: [i32; 3]) -> Option<FineSource<'_>> {
        let Some(stream) = &self.streaming else {
            return self.chunks.get(&key).map(FineSource::Stored);
        };
        // Procedural generation is defined only inside the simulation domain. Cells
        // outside it contribute air, even when a stored override covers them.
        if !Self::contains_stream_cell(key.map(|v| v * CHUNK_EDGE)) {
            return None;
        }
        match stream.overrides.get(&key) {
            // Preserve the tri-state: `Some(None)` is an explicit empty chunk and must
            // not fall through to legacy air or procedural generation.
            Some(chunk) => chunk.as_ref().map(FineSource::Stored),
            // Mirrors `stream_around`: a missing chunk in the legacy square is
            // snapshot-authoritative air, including deletions.
            None if (-2..2).contains(&key[0]) && (-2..2).contains(&key[2]) => None,
            None => generated_chunk(self.seed, key).map(FineSource::Generated),
        }
    }
}

enum FineSource<'a> {
    Stored(&'a Chunk),
    Generated(Chunk),
}

impl FineSource<'_> {
    fn voxels(&self) -> &[u8; CHUNK_VOLUME] {
        match self {
            Self::Stored(chunk) => &chunk.voxels,
            Self::Generated(chunk) => &chunk.voxels,
        }
    }

    fn solid(&self) -> usize {
        match self {
            Self::Stored(chunk) => chunk.solid,
            Self::Generated(chunk) => chunk.solid,
        }
    }
}

/// Fine-cell world box of a tile, or `None` when it is not representable in `i32`.
fn tile_origin(key: [i32; 3], span: i32) -> Option<[i32; 3]> {
    let mut origin = [0; 3];
    for (axis, &coordinate) in key.iter().enumerate() {
        let low = i64::from(coordinate) * i64::from(span);
        let high = low + i64::from(span) - 1;
        if low < i64::from(i32::MIN) || high > i64::from(i32::MAX) {
            return None;
        }
        origin[axis] = low as i32;
    }
    Some(origin)
}

/// Whether the tile's fine box overlaps the editable simulation domain.
fn intersects_simulation_domain(origin: [i32; 3], span: i32) -> bool {
    let domain = [
        (-WORLD_LIMIT, WORLD_LIMIT),
        (STREAM_MIN_Y, STREAM_MAX_Y),
        (-WORLD_LIMIT, WORLD_LIMIT),
    ];
    (0..3).all(|axis| {
        let low = i64::from(origin[axis]);
        let high = low + i64::from(span);
        low < i64::from(domain[axis].1) && i64::from(domain[axis].0) < high
    })
}

/// Material of the topmost occupied fine cell in one aggregate, by fixed scan order.
fn topmost_material(
    sources: &[Option<FineSource<'_>>],
    chunks_per_axis: usize,
    chunk_base: [i32; 3],
    base: [i32; 3],
    aggregation: i32,
) -> u8 {
    for dy in (0..aggregation).rev() {
        for dx in 0..aggregation {
            for dz in 0..aggregation {
                let material = sample(
                    sources,
                    chunks_per_axis,
                    chunk_base,
                    [base[0] + dx, base[1] + dy, base[2] + dz],
                );
                if material != 0 {
                    return material;
                }
            }
        }
    }
    0
}

/// Material at one fine cell from the preloaded sources of the tile.
fn sample(
    sources: &[Option<FineSource<'_>>],
    chunks_per_axis: usize,
    chunk_base: [i32; 3],
    cell: [i32; 3],
) -> u8 {
    let (chunk, index) = address(cell);
    let x = (chunk[0] - chunk_base[0]) as usize;
    let y = (chunk[1] - chunk_base[1]) as usize;
    let z = (chunk[2] - chunk_base[2]) as usize;
    sources[x + chunks_per_axis * (y + chunks_per_axis * z)]
        .as_ref()
        .map_or(0, |source| source.voxels()[index])
}

fn coarse_index(local: [i32; 3]) -> Option<usize> {
    if local
        .iter()
        .any(|&coordinate| !(0..COARSE_TILE_EDGE).contains(&coordinate))
    {
        return None;
    }
    Some((local[0] + COARSE_TILE_EDGE * (local[1] + COARSE_TILE_EDGE * local[2])) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    struct TempSave(PathBuf);
    impl TempSave {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "matterweave-coarse-{}-{}",
                std::process::id(),
                NEXT_TEST.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir.join("world.json"))
        }
    }
    impl Drop for TempSave {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    /// First solid fine cell of one aggregate in the documented selection order.
    fn topmost_witness(world: &World, base: [i32; 3], aggregation: i32) -> Option<[i32; 3]> {
        (0..aggregation)
            .rev()
            .flat_map(|dy| {
                (0..aggregation).flat_map(move |dx| (0..aggregation).map(move |dz| [dx, dy, dz]))
            })
            .map(|offset| {
                [
                    base[0] + offset[0],
                    base[1] + offset[1],
                    base[2] + offset[2],
                ]
            })
            .find(|&cell| world.get(cell) != 0)
    }

    #[test]
    fn negative_keys_partition_euclidean_tile_boundaries() {
        assert_eq!(tile_span(1), Some(32));
        assert_eq!(tile_span(2), Some(64));
        assert_eq!(tile_span(0), None);
        assert_eq!(tile_span(3), None);

        let mut world = World::new(3);
        assert!(world.set([-1, 0, -1], 3));
        let negative = world.coarse_tile(1, [-1, 0, -1]).unwrap();
        assert_eq!(negative.origin(), [-32, 0, -32]);
        assert_eq!(negative.key(), [-1, 0, -1]);
        assert_eq!(negative.span(), 32);
        // Fine -1 is the last cell of the Euclidean-negative tile, not of tile 0.
        assert_eq!(negative.material([15, 0, 15]), Some(3));
        assert_eq!(negative.world_cell([0, 0, 0]), Some([-32, 0, -32]));
        assert_eq!(negative.world_cell([16, 0, 0]), None);
        assert_eq!(
            negative.materials().len() as i32,
            COARSE_TILE_EDGE * COARSE_TILE_EDGE * COARSE_TILE_EDGE
        );
        assert!(world.coarse_tile(1, [0, 0, 0]).unwrap().is_empty());

        let mut boundary = World::new(3);
        assert!(boundary.set([-64, 0, -64], 5));
        assert!(boundary.set([-65, 0, 0], 6));
        assert_eq!(
            boundary
                .coarse_tile(2, [-1, 0, -1])
                .unwrap()
                .material([0, 0, 0]),
            Some(5)
        );
        assert_eq!(
            boundary
                .coarse_tile(2, [-2, 0, 0])
                .unwrap()
                .material([15, 0, 0]),
            Some(6)
        );
    }

    #[test]
    fn world_cell_maps_local_coarse_steps_by_level() {
        let world = World::new(3);
        // Level 1: each local step is 2 fine cells, from both positive and negative
        // Euclidean origins.
        let positive = world.coarse_tile(1, [2, 0, 3]).unwrap();
        assert_eq!(positive.origin(), [64, 0, 96]);
        assert_eq!(positive.world_cell([1, 0, 2]), Some([66, 0, 100]));
        assert_eq!(positive.world_cell([15, 15, 15]), Some([94, 30, 126]));
        let negative = world.coarse_tile(1, [-2, -1, -3]).unwrap();
        assert_eq!(negative.origin(), [-64, -32, -96]);
        assert_eq!(negative.world_cell([1, 0, 2]), Some([-62, -32, -92]));
        assert_eq!(negative.world_cell([0, 15, 15]), Some([-64, -2, -66]));
        assert_eq!(negative.world_cell([16, 0, 0]), None);

        // Level 2: each local step is 4 fine cells.
        let positive = world.coarse_tile(2, [1, 0, 1]).unwrap();
        assert_eq!(positive.origin(), [64, 0, 64]);
        assert_eq!(positive.world_cell([1, 2, 3]), Some([68, 8, 76]));
        let negative = world.coarse_tile(2, [-1, -1, -1]).unwrap();
        assert_eq!(negative.origin(), [-64, -64, -64]);
        assert_eq!(negative.world_cell([1, 2, 3]), Some([-60, -56, -52]));
        assert_eq!(negative.world_cell([15, 15, 15]), Some([-4, -4, -4]));
    }

    #[test]
    fn occupancy_is_conservative_and_fills_thin_holes() {
        let mut world = World::new(0);
        for x in 0..2 {
            for y in 0..2 {
                for z in 0..2 {
                    assert!(world.set([x, y, z], 3));
                }
            }
        }
        assert!(world.set([0, 1, 0], 0));
        assert_eq!(world.get([0, 1, 0]), 0);
        // An isolated cell must survive level-2 aggregation into an empty block.
        assert!(world.set([7, 7, 7], 6));

        let level_one = world.coarse_tile(1, [0, 0, 0]).unwrap();
        assert_eq!(level_one.material([0, 0, 0]), Some(3));
        assert_eq!(level_one.material([3, 3, 3]), Some(6));
        assert_eq!(level_one.solid_cells(), 2);

        let level_two = world.coarse_tile(2, [0, 0, 0]).unwrap();
        assert_eq!(level_two.material([0, 0, 0]), Some(3));
        assert_eq!(level_two.material([1, 1, 1]), Some(6));
        assert_eq!(level_two.solid_cells(), 2);
    }

    #[test]
    fn material_choice_is_topmost_then_scan_order() {
        let mut world = World::new(0);
        assert!(world.set([0, 0, 0], 3));
        assert!(world.set([1, 1, 1], 5));
        assert!(world.set([0, 2, 0], 2));
        assert!(world.set([1, 2, 1], 4));

        let tile = world.coarse_tile(1, [0, 0, 0]).unwrap();
        // Level 1: the topmost cell of aggregate [0, 2)^3 is [1, 1, 1].
        assert_eq!(tile.material([0, 0, 0]), Some(5));
        // Ties at the topmost height go to the lowest x, then lowest z.
        assert_eq!(tile.material([0, 1, 0]), Some(2));
        // Level 2 widens the aggregate: the same material policy applies over 4^3 cells.
        assert_eq!(
            world.coarse_tile(2, [0, 0, 0]).unwrap().material([0, 0, 0]),
            Some(2)
        );
        assert_eq!(tile, world.coarse_tile(1, [0, 0, 0]).unwrap());
    }

    #[test]
    fn streaming_legacy_air_and_tombstones_survive_eviction_and_reload() {
        let mut world = World::new(11);
        world.enable_streaming();
        // A fresh streaming world's legacy square is snapshot-authoritative air even
        // though the procedural generator would emit solid terrain there: every
        // column in fine [0, 32) has generated height >= 0.
        assert!(world.coarse_tile(1, [0, 0, 0]).unwrap().is_empty());
        assert!(world.stream_around([64.0, 0.0, 64.0]));
        let before = world.coarse_tile(1, [2, 0, 2]).unwrap();
        assert!(before.solid_cells() > 0);

        // Delete every solid cell of one procedural chunk: an explicit empty override.
        let mut cleared = 0;
        for x in 64..80 {
            for y in 0..16 {
                for z in 64..80 {
                    if world.get([x, y, z]) != 0 {
                        assert!(world.set([x, y, z], 0));
                        cleared += 1;
                    }
                }
            }
        }
        assert!(cleared > 0);
        let after = world.coarse_tile(1, [2, 0, 2]).unwrap();
        // The tombstoned chunk is the 8x8x8 coarse block at the tile origin; the
        // neighbouring procedural chunks in the tile are untouched.
        for cy in 0..8 {
            for cz in 0..8 {
                for cx in 0..8 {
                    assert_eq!(after.material([cx, cy, cz]), Some(0));
                }
            }
        }
        assert!(after.solid_cells() < before.solid_cells());

        // The tombstone survives eviction and an atomic save/reload.
        assert!(world.stream_around([-150.0, 5.0, 0.0]));
        assert_eq!(
            world.coarse_tile(1, [2, 0, 2]).unwrap().materials(),
            after.materials()
        );
        let save = TempSave::new();
        world.save(&save.0).unwrap();
        let loaded = World::load(&save.0).unwrap();
        assert!(loaded.is_streaming());
        assert_eq!(
            loaded.coarse_tile(1, [2, 0, 2]).unwrap().materials(),
            after.materials()
        );
    }

    #[test]
    fn level_two_tile_classifies_each_fine_chunk_for_legacy_air() {
        let mut world = World::new(13);
        world.enable_streaming();
        // Tile [0, 0, 0] at level 2 spans fine chunks 0..4 per axis. Chunks 0..2 are
        // the snapshot-authoritative legacy square; chunks 2..4 are procedural and
        // solid at these heights. Classifying the whole tile once would fill or clear
        // one half, so the rule must be applied per fine chunk.
        let tile = world.coarse_tile(2, [0, 0, 0]).unwrap();
        let mut procedural = 0;
        for cy in 0..COARSE_TILE_EDGE {
            for cz in 0..COARSE_TILE_EDGE {
                for cx in 0..COARSE_TILE_EDGE {
                    let material = tile.material([cx, cy, cz]).unwrap();
                    if cx < 8 && cz < 8 {
                        assert_eq!(material, 0, "legacy corner at {cx},{cy},{cz}");
                    } else if material != 0 {
                        procedural += 1;
                    }
                }
            }
        }
        assert!(
            procedural > 0,
            "the procedural half of the tile must contribute terrain"
        );
    }

    #[test]
    fn edits_survive_eviction_save_and_reload() {
        let mut world = World::new(12);
        world.enable_streaming();
        assert!(world.stream_around([64.0, 0.0, 64.0]));
        // Pick the fine cell that decides one coarse material, then edit it.
        let mut chosen = None;
        'search: for cz in 0..8 {
            for cy in 0..8 {
                for cx in 0..8 {
                    let base = [64 + 2 * cx, 2 * cy, 64 + 2 * cz];
                    if let Some(cell) = topmost_witness(&world, base, 2) {
                        chosen = Some((cell, [cx, cy, cz]));
                        break 'search;
                    }
                }
            }
        }
        let (witness, local) = chosen.expect("generated chunk must contain solid terrain");
        assert!(world.set(witness, 7));
        let edited = world.coarse_tile(1, [2, 0, 2]).unwrap();
        assert_eq!(edited.material(local), Some(7));
        assert_eq!(edited.source_revision(), world.revision());

        // Eviction removes the fine chunk from residency, so World::get alone is not
        // enough; the stored override must keep the derived tile unchanged.
        assert!(world.stream_around([-150.0, 5.0, 0.0]));
        assert_eq!(world.get(witness), 0);
        let evicted = world.coarse_tile(1, [2, 0, 2]).unwrap();
        assert_eq!(evicted.materials(), edited.materials());
        assert_eq!(evicted.source_revision(), world.revision());

        let save = TempSave::new();
        world.save(&save.0).unwrap();
        let loaded = World::load(&save.0).unwrap();
        assert!(loaded.is_streaming());
        assert_eq!(loaded.get(witness), 0);
        let reloaded = loaded.coarse_tile(1, [2, 0, 2]).unwrap();
        assert_eq!(reloaded.materials(), edited.materials());
        assert_eq!(reloaded.seed(), world.seed());
        assert_eq!(reloaded.source_revision(), world.revision());
        assert_eq!(reloaded.generator_version(), edited.generator_version());
    }

    #[test]
    fn partial_level_two_tiles_treat_outside_domain_as_air() {
        let mut stored = World::new(5);
        assert!(stored.set([0, 3, 0], 3)); // inside the streaming Y domain
        assert!(stored.set([0, 40, 0], 3)); // above it, still a legal stored coordinate
        stored.enable_streaming();

        let inside = stored.coarse_tile(2, [0, 0, 0]).unwrap();
        assert_eq!(inside.origin(), [0, 0, 0]);
        assert_eq!(inside.material([0, 0, 0]), Some(3));
        // Fine y 40 maps to coarse y 10, outside the simulation domain: the stored
        // override there must not contribute, while procedural terrain in-domain does.
        assert_eq!(inside.material([0, 10, 0]), Some(0));
        for cy in 8..COARSE_TILE_EDGE {
            for cz in 0..COARSE_TILE_EDGE {
                for cx in 0..COARSE_TILE_EDGE {
                    assert_eq!(inside.material([cx, cy, cz]), Some(0));
                }
            }
        }
        assert!(inside.solid_cells() > 0);

        // A partially intersecting level-2 tile is valid: only fine y -16..0 is in
        // the domain, and every fully out-of-domain coarse cell is air.
        let below = stored.coarse_tile(2, [0, -1, 0]).unwrap();
        assert_eq!(below.origin(), [0, -64, 0]);
        for cy in 0..12 {
            for cz in 0..COARSE_TILE_EDGE {
                for cx in 0..COARSE_TILE_EDGE {
                    assert_eq!(below.material([cx, cy, cz]), Some(0));
                }
            }
        }

        // A wholly disjoint Y key is rejected.
        assert_eq!(
            stored.coarse_tile(2, [0, 1, 0]),
            Err(CoarseError::OutsideSupportedDomain {
                level: 2,
                key: [0, 1, 0]
            })
        );
        assert_eq!(
            stored.coarse_tile(2, [0, -2, 0]),
            Err(CoarseError::OutsideSupportedDomain {
                level: 2,
                key: [0, -2, 0]
            })
        );
    }

    #[test]
    fn nonstreaming_uses_stored_data_at_arbitrary_coordinates() {
        let mut world = World::new(3);
        assert!(world.set([1000, 5, -1000], 3));
        let tile = world
            .coarse_tile(1, [1000i32.div_euclid(32), 0, (-1000i32).div_euclid(32)])
            .unwrap();
        assert_eq!(tile.origin(), [992, 0, -1024]);
        assert_eq!(tile.material([4, 2, 12]), Some(3));
        assert_eq!(tile.solid_cells(), 1);

        // A representable request with no stored data is a valid empty tile.
        assert!(world.coarse_tile(2, [1000, 0, -1000]).unwrap().is_empty());

        // The same request on a streaming world is explicitly out of domain.
        world.enable_streaming();
        assert_eq!(
            world.coarse_tile(1, [31, 0, -32]),
            Err(CoarseError::OutsideSupportedDomain {
                level: 1,
                key: [31, 0, -32]
            })
        );
    }

    #[test]
    fn levels_keys_and_overflow_are_rejected_explicitly() {
        let mut streaming = World::new(1);
        streaming.enable_streaming();
        let nonstreaming = World::new(1);
        for level in [0u8, 3, 255] {
            assert_eq!(
                streaming.coarse_tile(level, [0, 0, 0]),
                Err(CoarseError::UnsupportedLevel { level })
            );
            assert_eq!(
                nonstreaming.coarse_tile(level, [0, 0, 0]),
                Err(CoarseError::UnsupportedLevel { level })
            );
        }

        // Partially intersecting tiles are legal at both levels.
        assert!(streaming.coarse_tile(1, [0, -1, 0]).is_ok());
        assert!(streaming.coarse_tile(2, [0, -1, 0]).is_ok());
        assert!(streaming.coarse_tile(2, [0, 0, 0]).is_ok());
        for (level, key) in [
            (1u8, [0, 1, 0]),
            (1, [0, -2, 0]),
            (1, [8, 0, 0]),
            (1, [-9, 0, 0]),
            (2, [0, 1, 0]),
            (2, [0, -2, 0]),
            (2, [4, 0, 0]),
            (2, [-5, 0, 0]),
            (1, [1000, 0, -1000]),
        ] {
            assert_eq!(
                streaming.coarse_tile(level, key),
                Err(CoarseError::OutsideSupportedDomain { level, key }),
                "level {level} key {key:?}"
            );
            // Non-streaming worlds have no simulation-domain limit.
            assert!(
                nonstreaming.coarse_tile(level, key).is_ok(),
                "level {level} key {key:?}"
            );
        }

        // Checked coordinate arithmetic rejects overflow instead of wrapping.
        for key in [
            [i32::MAX, 0, 0],
            [i32::MIN, 0, 0],
            [0, i32::MAX, 0],
            [0, 0, i32::MIN],
        ] {
            let error = CoarseError::OutOfRange { level: 1, key };
            assert_eq!(streaming.coarse_tile(1, key), Err(error));
            assert_eq!(nonstreaming.coarse_tile(1, key), Err(error));
        }
        let max_level_two = i32::MAX / 64;
        assert!(nonstreaming.coarse_tile(2, [max_level_two, 0, 0]).is_ok());
        assert_eq!(
            nonstreaming.coarse_tile(2, [max_level_two + 1, 0, 0]),
            Err(CoarseError::OutOfRange {
                level: 2,
                key: [max_level_two + 1, 0, 0]
            })
        );
        let min_level_two = i32::MIN / 64;
        assert!(nonstreaming.coarse_tile(2, [0, 0, min_level_two]).is_ok());
        assert_eq!(
            nonstreaming.coarse_tile(2, [0, 0, min_level_two - 1]),
            Err(CoarseError::OutOfRange {
                level: 2,
                key: [0, 0, min_level_two - 1]
            })
        );
    }

    #[test]
    fn derivation_is_deterministic_and_does_not_mutate() {
        let mut world = World::new(2);
        assert!(world.set([0, 0, 0], 1));
        assert!(world.set([5, 5, 5], 2));
        let stats = world.stats();
        let revision = world.revision();
        let first = world.coarse_tile(1, [0, 0, 0]).unwrap();
        let _ = world.coarse_tile(2, [0, 0, 0]).unwrap();
        assert!(world.coarse_tile(1, [1000, 0, 0]).unwrap().is_empty());
        let second = world.coarse_tile(1, [0, 0, 0]).unwrap();
        assert_eq!(first, second);
        assert_eq!(world.stats(), stats);
        assert_eq!(world.revision(), revision);
        assert_eq!(world.get([0, 0, 0]), 1);

        let mut streaming = World::new(2);
        streaming.enable_streaming();
        assert!(streaming.stream_around([64.0, 0.0, 64.0]));
        let stats = streaming.stats();
        let tile = streaming.coarse_tile(1, [2, 0, 2]).unwrap();
        assert_eq!(streaming.coarse_tile(1, [2, 0, 2]).unwrap(), tile);
        assert_eq!(streaming.stats(), stats);
    }

    #[test]
    fn every_generated_solid_cell_maps_into_an_occupied_coarse_cell() {
        let mut world = World::new(7);
        world.enable_streaming();
        assert!(world.stream_around([64.0, 0.0, 64.0]));
        let level_one = world.coarse_tile(1, [2, 0, 2]).unwrap();
        let level_two = world.coarse_tile(2, [1, 0, 1]).unwrap();
        assert_eq!(level_one.origin(), [64, 0, 64]);
        assert_eq!(level_two.origin(), [64, 0, 64]);

        let mut solids = 0;
        for x in 64..96 {
            for y in 0..16 {
                for z in 64..96 {
                    if world.get([x, y, z]) == 0 {
                        continue;
                    }
                    solids += 1;
                    let local = [x - 64, y, z - 64];
                    assert_ne!(
                        level_one.material(local.map(|v| v / 2)),
                        Some(0),
                        "level 1 lost solid cell {x},{y},{z}"
                    );
                    assert_ne!(
                        level_two.material(local.map(|v| v / 4)),
                        Some(0),
                        "level 2 lost solid cell {x},{y},{z}"
                    );
                }
            }
        }
        assert!(solids > 0);
    }
}
