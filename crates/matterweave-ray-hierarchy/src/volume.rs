//! Derived bounded snapshot: material words, packed occupancy and source provenance.
//!
//! The authoritative voxel data stays in [`matterweave_core::World`]. This snapshot is
//! derived, versioned and never written back, following
//! [ADR-0005](../../../docs/adr/0005-voxel-world-data.md).

use crate::occupancy::{BitUpdate, BlockShape, OccupancyGrid};
use matterweave_core::World;
use matterweave_render::ray_reference::RayVolume;
use std::fmt;

/// Inclusive local-coordinate bound mirrored from `RayVolume::pack`: a crop endpoint
/// must lie within `-MAX_COORD..=MAX_COORD`.
pub const MAX_COORD: i32 = 8192;

/// Rejected requests, all detected before any traversal runs.
#[derive(Clone, Debug, PartialEq)]
pub enum HierarchyError {
    /// A crop axis is zero or exceeds `MAX_AXIS` (128).
    VolumeDimensions { dimensions: [u32; 3] },
    /// The crop exceeds `MAX_CELLS` (64^3) cells.
    VolumeCells,
    /// The material word count does not match the crop cell count.
    MaterialCount { expected: usize, actual: usize },
    /// A material word exceeds `u8::MAX`, which hits report as [`matterweave_core::RayHit`].
    MaterialOutOfRange { index: usize, material: u32 },
    /// A crop endpoint falls outside `+/-MAX_COORD`.
    VolumeBounds {
        origin: [i32; 3],
        dimensions: [u32; 3],
    },
    /// A block coordinate is outside the packed grid; an internal consistency failure.
    BlockOutOfRange { block: [u32; 3] },
    /// A requested cell is outside the snapshot's half-open crop.
    CellOutsideVolume { cell: [i32; 3] },
    /// A ray direction was nonfinite, zero or not unit length within 1e-6.
    InvalidDirection { direction: [f64; 3] },
    /// A ray origin or distance was nonfinite, or the distance left `0..=MAX_RAY_DISTANCE`.
    InvalidRay { max_distance: f64 },
    /// A fallible allocation failed.
    Allocation { what: &'static str },
}

impl fmt::Display for HierarchyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VolumeDimensions { dimensions } => {
                write!(
                    f,
                    "crop dimensions {dimensions:?} are outside 1..=128 per axis"
                )
            }
            Self::VolumeCells => write!(f, "crop exceeds 64^3 cells"),
            Self::MaterialCount { expected, actual } => {
                write!(f, "material words {actual} do not match {expected} cells")
            }
            Self::MaterialOutOfRange { index, material } => {
                write!(f, "material {material} at {index} exceeds u8::MAX")
            }
            Self::VolumeBounds { origin, dimensions } => write!(
                f,
                "crop origin {origin:?} and dimensions {dimensions:?} exceed +/-{MAX_COORD}"
            ),
            Self::BlockOutOfRange { block } => write!(f, "block {block:?} is outside the grid"),
            Self::CellOutsideVolume { cell } => write!(f, "cell {cell:?} is outside the crop"),
            Self::InvalidDirection { direction } => {
                write!(
                    f,
                    "direction {direction:?} is not finite, nonzero and unit length"
                )
            }
            Self::InvalidRay { max_distance } => write!(
                f,
                "ray distance {max_distance} is outside 0..={}",
                matterweave_core::MAX_RAY_DISTANCE
            ),
            Self::Allocation { what } => write!(f, "allocation of {what} failed"),
        }
    }
}

impl std::error::Error for HierarchyError {}

/// Validates crop dimensions and returns the cell count, mirroring `RayVolume::pack`.
pub fn cells_in(dimensions: [u32; 3]) -> Result<usize, HierarchyError> {
    const MAX_AXIS: u32 = matterweave_render::ray_reference::MAX_AXIS;
    const MAX_CELLS: usize = matterweave_render::ray_reference::MAX_CELLS;
    if dimensions.iter().any(|&d| d == 0 || d > MAX_AXIS) {
        return Err(HierarchyError::VolumeDimensions { dimensions });
    }
    dimensions
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d as usize))
        .filter(|&n| n <= MAX_CELLS)
        .ok_or(HierarchyError::VolumeCells)
}

/// Provenance of a derived snapshot: the same key `RayVolume` validates against.
///
/// Epochs are caller-owned and must change on replacement, load or fork even at equal
/// seed and revision, because `World` carries no instance identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceKey {
    pub epoch: u64,
    pub revision: u64,
    pub seed: u64,
}

impl SourceKey {
    /// Reads the key of an existing reference pack.
    pub fn from_reference(reference: &RayVolume) -> Self {
        Self {
            epoch: reference.source_epoch(),
            revision: reference.source_revision(),
            seed: reference.source_seed(),
        }
    }
}

/// Logical payload sizes, not allocator overhead, staging or resident memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryStats {
    pub cells: usize,
    /// Dense material words, four bytes per cell.
    pub material_bytes: usize,
    /// Packed occupancy words kept beside the materials.
    pub occupancy_words: usize,
    pub occupancy_bytes: usize,
    pub total_bytes: usize,
}

/// Structural build cost of one snapshot: what the constructor touches, exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildReport {
    pub cells: usize,
    /// Material words copied from the source crop, one per cell.
    pub material_words_copied: usize,
    pub occupancy_blocks: usize,
    /// Occupancy words allocated and zeroed, `blocks * words_per_block`.
    pub occupancy_words_allocated: usize,
    /// Solid bits set, which equals the crop's solid cell count.
    pub occupancy_bits_set: usize,
}

/// Bounded write report of one [`HierarchyVolume::patch_cell`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatchReport {
    pub material_changed: bool,
    pub occupancy_bit_changed: bool,
    /// Occupancy words written: zero for an unchanged request, otherwise exactly one.
    pub occupancy_words_written: usize,
    pub block_became_empty: bool,
    pub block_became_occupied: bool,
}

/// Immutable-in-world, mutable-in-memory derived traversal snapshot.
#[derive(Debug)]
pub struct HierarchyVolume {
    origin: [i32; 3],
    dimensions: [u32; 3],
    materials: Vec<u32>,
    occupancy: OccupancyGrid,
    key: SourceKey,
    locally_edited: bool,
    material_words_copied: usize,
}

impl HierarchyVolume {
    /// Copies a validated reference pack into a hierarchy snapshot.
    ///
    /// The pack's crop rules, bounds and provenance are reused unchanged; the material
    /// words are copied once so a later [`Self::patch_cell`] cannot touch the source.
    pub fn from_reference(
        reference: &RayVolume,
        shape: BlockShape,
    ) -> Result<Self, HierarchyError> {
        Self::build(
            reference.origin(),
            reference.dimensions(),
            reference.materials(),
            SourceKey::from_reference(reference),
            shape,
        )
    }

    /// Builds a snapshot from raw crop data, applying the same bounded crop rules as
    /// `RayVolume::pack` so a pack-valid crop is always accepted.
    ///
    /// Materials are `u32` per cell, x fastest, air zero. Values above `u8::MAX` are
    /// rejected because hits report `matterweave_core::RayHit` material bytes.
    pub fn build(
        origin: [i32; 3],
        dimensions: [u32; 3],
        materials: &[u32],
        key: SourceKey,
        shape: BlockShape,
    ) -> Result<Self, HierarchyError> {
        let cells = cells_in(dimensions)?;
        for axis in 0..3 {
            let end = i64::from(origin[axis]) + i64::from(dimensions[axis]);
            if origin[axis] < -MAX_COORD || end > i64::from(MAX_COORD) {
                return Err(HierarchyError::VolumeBounds { origin, dimensions });
            }
        }
        if materials.len() != cells {
            return Err(HierarchyError::MaterialCount {
                expected: cells,
                actual: materials.len(),
            });
        }
        for (index, &material) in materials.iter().enumerate() {
            if material > u32::from(u8::MAX) {
                return Err(HierarchyError::MaterialOutOfRange { index, material });
            }
        }
        let occupancy = OccupancyGrid::build(dimensions, shape, materials)?;
        let mut copied = Vec::new();
        copied
            .try_reserve_exact(cells)
            .map_err(|_| HierarchyError::Allocation {
                what: "material words",
            })?;
        copied.extend_from_slice(materials);
        let material_words_copied = copied.len();
        Ok(Self {
            origin,
            dimensions,
            materials: copied,
            occupancy,
            key,
            locally_edited: false,
            material_words_copied,
        })
    }

    /// Local minimum cell of the half-open crop.
    pub fn origin(&self) -> [i32; 3] {
        self.origin
    }

    /// Crop extent in cells.
    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }

    /// Packed occupancy block shape.
    pub fn shape(&self) -> BlockShape {
        self.occupancy.shape()
    }

    /// Dense material words, one `u32` per cell, x fastest.
    pub fn materials(&self) -> &[u32] {
        &self.materials
    }

    /// Packed occupancy kept beside the materials, never instead of them.
    pub fn occupancy(&self) -> &OccupancyGrid {
        &self.occupancy
    }

    /// Provenance key of the source crop.
    pub fn key(&self) -> SourceKey {
        self.key
    }

    /// Caller-owned epoch of the derived snapshot; parallel to `RayVolume::source_epoch`.
    pub fn source_epoch(&self) -> u64 {
        self.key.epoch
    }

    /// Whole-world revision at derivation time; parallel to `RayVolume::source_revision`.
    pub fn source_revision(&self) -> u64 {
        self.key.revision
    }

    /// Seed of the world this snapshot was derived from; parallel to
    /// `RayVolume::source_seed`.
    pub fn source_seed(&self) -> u64 {
        self.key.seed
    }

    /// Whether [`Self::patch_cell`] has changed this snapshot away from its source key.
    pub fn locally_edited(&self) -> bool {
        self.locally_edited
    }

    /// Whether this snapshot still describes `world` at `epoch`.
    ///
    /// The rule is identical to `RayVolume::valid_for` for unedited snapshots: epoch,
    /// whole-world revision and seed must all match, so any edit anywhere invalidates
    /// the crop. A locally patched snapshot is never valid for a source world: its
    /// material and occupancy words no longer come from one published revision.
    pub fn valid_for(&self, world: &World, epoch: u64) -> bool {
        !self.locally_edited
            && self.key.epoch == epoch
            && self.key.revision == world.revision()
            && self.key.seed == world.seed()
    }

    /// Material of a world cell, or `None` when the cell is outside the crop.
    pub fn material_at(&self, cell: [i32; 3]) -> Option<u32> {
        let local = self.local_cell(cell)?;
        let [x, y, z] = local;
        let [dx, dy, _] = self.dimensions;
        let index = (x + dx * (y + dy * z)) as usize;
        self.materials.get(index).copied()
    }

    /// Logical payload sizes of the snapshot.
    pub fn memory_stats(&self) -> MemoryStats {
        let material_bytes = self.materials.len() * size_of::<u32>();
        let occupancy_bytes = self.occupancy.bytes();
        MemoryStats {
            cells: self.materials.len(),
            material_bytes,
            occupancy_words: self.occupancy.words().len(),
            occupancy_bytes,
            total_bytes: material_bytes + occupancy_bytes,
        }
    }

    /// Structural build cost actually incurred by the constructor.
    pub fn build_report(&self) -> BuildReport {
        BuildReport {
            cells: self.materials.len(),
            material_words_copied: self.material_words_copied,
            occupancy_blocks: self.occupancy.blocks(),
            occupancy_words_allocated: self.occupancy.words().len(),
            occupancy_bits_set: self.occupancy.bits_set(),
        }
    }

    /// Writes one derived cell and its occupancy bit.
    ///
    /// This is the bounded snapshot-patch path: at most one material word and one
    /// occupancy word change, and the snapshot becomes stale for its source world.
    /// Authoritative voxel data is untouched; a caller that needs a fresh snapshot
    /// derives a new one from the edited world.
    pub fn patch_cell(
        &mut self,
        cell: [i32; 3],
        material: u8,
    ) -> Result<PatchReport, HierarchyError> {
        let local = self
            .local_cell(cell)
            .ok_or(HierarchyError::CellOutsideVolume { cell })?;
        let [x, y, z] = local;
        let [dx, dy, _] = self.dimensions;
        let index = (x + dx * (y + dy * z)) as usize;
        let word = u32::from(material);
        let material_changed = self.materials.get(index) != Some(&word);
        let update = if material_changed {
            if let Some(slot) = self.materials.get_mut(index) {
                *slot = word;
            }
            self.occupancy.set_cell(local, material != 0)
        } else {
            None
        };
        let bit_changed = update.is_some_and(|u: BitUpdate| u.changed);
        let report = PatchReport {
            material_changed,
            occupancy_bit_changed: bit_changed,
            occupancy_words_written: usize::from(bit_changed),
            // Only a bit change can empty or occupy a block: patching one solid material
            // for another, or rewriting the same value, leaves occupancy untouched.
            block_became_empty: bit_changed && update.is_some_and(|u| u.block_empty_after),
            block_became_occupied: bit_changed
                && update.is_some_and(|u| u.block_empty_before && !u.block_empty_after),
        };
        if material_changed {
            self.locally_edited = true;
        }
        Ok(report)
    }

    fn local_cell(&self, cell: [i32; 3]) -> Option<[u32; 3]> {
        let mut local = [0u32; 3];
        for axis in 0..3 {
            let offset = i64::from(cell[axis]) - i64::from(self.origin[axis]);
            if offset < 0 || offset >= i64::from(self.dimensions[axis]) {
                return None;
            }
            local[axis] = offset as u32;
        }
        Some(local)
    }
}

/// Depth of a hit position, mirroring the reference fragment stage's arithmetic.
///
/// `world = origin + direction * distance` in `f32`, then `clip.z / clip.w` for a
/// column-major projection. Returns `None` for a nonfinite matrix product or a zero
/// `w`. The matrix is a caller precondition: finite, invertible, positive near,
/// ordinary (not reversed) `0..1` depth.
pub fn clip_depth(
    view_projection: &[[f32; 4]; 4],
    origin: [f64; 3],
    direction: [f64; 3],
    distance: f64,
) -> Option<f32> {
    let world = [
        origin[0] as f32 + direction[0] as f32 * distance as f32,
        origin[1] as f32 + direction[1] as f32 * distance as f32,
        origin[2] as f32 + direction[2] as f32 * distance as f32,
    ];
    let column = |index: usize| view_projection[index];
    let dot = |row: usize| {
        column(0)[row] * world[0]
            + column(1)[row] * world[1]
            + column(2)[row] * world[2]
            + column(3)[row]
    };
    let z = dot(2);
    let w = dot(3);
    if !z.is_finite() || !w.is_finite() || w == 0.0 {
        return None;
    }
    let depth = z / w;
    depth.is_finite().then_some(depth)
}
