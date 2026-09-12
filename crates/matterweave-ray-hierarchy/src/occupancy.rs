//! Packed voxel occupancy: one bit per cell, grouped in fixed blocks and kept
//! separately from the material words.
//!
//! See the crate documentation for the layout contract and the provenance of the
//! block-reuse idea.

use crate::{cells_in, HierarchyError};

/// Largest block the packed words may describe: 512 bits = 8x8x8 cells.
///
/// The bound keeps one block's occupancy words inside 16 `u32` words, so a traversal
/// can hold the current block's words in registers. [`BlockShape::CUBE4`],
/// [`BlockShape::TALL_4_4_8`] and [`BlockShape::CUBE8`] are inside the bound.
pub const MAX_BLOCK_BITS: usize = 512;

/// Block extent in cells, one occupancy bit per covered cell.
///
/// Values only exist through [`BlockShape::new`] or the named constants, so every
/// shape has nonzero axes of at most
/// [`MAX_AXIS`](matterweave_render::ray_reference::MAX_AXIS) and at most
/// [`MAX_BLOCK_BITS`] packed bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockShape {
    x: u32,
    y: u32,
    z: u32,
}

impl BlockShape {
    /// 4x4x4 cells: 64 bits, the finest granularity tested here.
    pub const CUBE4: Self = Self { x: 4, y: 4, z: 4 };
    /// 4x4x8 cells: 128 bits, the block shape used by the cited traversal reference.
    pub const TALL_4_4_8: Self = Self { x: 4, y: 4, z: 8 };
    /// 8x8x8 cells: 512 bits, the coarsest tested granularity.
    pub const CUBE8: Self = Self { x: 8, y: 8, z: 8 };

    /// Validates a shape: nonzero axes, each at most
    /// [`MAX_AXIS`](matterweave_render::ray_reference::MAX_AXIS), at most
    /// [`MAX_BLOCK_BITS`] packed bits. `None` describes the rejected request.
    pub const fn new(x: u32, y: u32, z: u32) -> Option<Self> {
        const MAX_AXIS: u32 = matterweave_render::ray_reference::MAX_AXIS;
        if x == 0 || y == 0 || z == 0 || x > MAX_AXIS || y > MAX_AXIS || z > MAX_AXIS {
            return None;
        }
        if (x as usize) * (y as usize) * (z as usize) > MAX_BLOCK_BITS {
            return None;
        }
        Some(Self { x, y, z })
    }

    /// Extent in cells.
    pub const fn dims(self) -> [u32; 3] {
        [self.x, self.y, self.z]
    }

    /// Packed bits per block, `x * y * z`.
    pub const fn bits(self) -> usize {
        (self.x as usize) * (self.y as usize) * (self.z as usize)
    }

    /// `u32` words per block, `ceil(bits / 32)`.
    pub const fn words_per_block(self) -> usize {
        self.bits().div_ceil(32)
    }

    /// Bit position of a local cell, `x + sx * (y + sy * z)`.
    ///
    /// The layout generalizes the cited reference's 4x4x8 packing: the bits of one block
    /// live in consecutive words, low bits first, with x fastest.
    pub const fn bit_index(self, local: [u32; 3]) -> usize {
        (local[0] + self.x * (local[1] + self.y * local[2])) as usize
    }
}

/// A block's occupancy bit and the write it caused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitUpdate {
    /// Whether the requested value differed from the stored bit.
    pub changed: bool,
    /// Word index inside [`OccupancyGrid::words`] this update wrote.
    pub word: usize,
    /// Whether the block held no solid cell before the update.
    pub block_empty_before: bool,
    /// Whether the block holds no solid cell after the update.
    pub block_empty_after: bool,
}

/// Fixed blocks of packed occupancy bits over a bounded crop, without material data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupancyGrid {
    shape: BlockShape,
    dimensions: [u32; 3],
    block_dims: [u32; 3],
    words: Vec<u32>,
    bits_set: usize,
}

impl OccupancyGrid {
    /// Packs `materials` (x fastest, one `u32` per cell) into block bits.
    ///
    /// Air is material zero; every other value is solid. The caller has already
    /// validated the crop, so this rejects only a dimension, length or allocation
    /// failure. Every solid cell sets exactly one bit and [`Self::bits_set`] counts
    /// those changes, so the packing cost is bounded by one scan of `materials` plus
    /// `blocks * words_per_block` words of storage.
    pub fn build(
        dimensions: [u32; 3],
        shape: BlockShape,
        materials: &[u32],
    ) -> Result<Self, HierarchyError> {
        let cells = cells_in(dimensions)?;
        if materials.len() != cells {
            return Err(HierarchyError::MaterialCount {
                expected: cells,
                actual: materials.len(),
            });
        }
        let shape_dims = shape.dims();
        let block_dims = [
            dimensions[0].div_ceil(shape_dims[0]),
            dimensions[1].div_ceil(shape_dims[1]),
            dimensions[2].div_ceil(shape_dims[2]),
        ];
        let blocks = block_count(block_dims)?;
        let words_len = blocks
            .checked_mul(shape.words_per_block())
            .ok_or(HierarchyError::VolumeCells)?;
        let mut words = Vec::new();
        words
            .try_reserve_exact(words_len)
            .map_err(|_| HierarchyError::Allocation {
                what: "occupancy words",
            })?;
        words.resize(words_len, 0u32);
        let words_per_block = shape.words_per_block();
        let mut bits_set = 0usize;
        let row = dimensions[0] as usize;
        let plane = row * dimensions[1] as usize;
        for (index, &material) in materials.iter().enumerate() {
            if material == 0 {
                continue;
            }
            let cell = [
                (index % row) as u32,
                ((index / row) % dimensions[1] as usize) as u32,
                (index / plane) as u32,
            ];
            let block = [
                cell[0] / shape_dims[0],
                cell[1] / shape_dims[1],
                cell[2] / shape_dims[2],
            ];
            let local = [
                cell[0] % shape_dims[0],
                cell[1] % shape_dims[1],
                cell[2] % shape_dims[2],
            ];
            let Some(block_index) = block_index(block, block_dims) else {
                return Err(HierarchyError::BlockOutOfRange { block });
            };
            let bit = shape.bit_index(local);
            let word = block_index * words_per_block + bit / 32;
            let mask = 1u32 << (bit % 32);
            if let Some(slot) = words.get_mut(word) {
                if *slot & mask == 0 {
                    *slot |= mask;
                    bits_set += 1;
                }
            }
        }
        Ok(Self {
            shape,
            dimensions,
            block_dims,
            words,
            bits_set,
        })
    }

    /// Block extent in cells.
    pub fn shape(&self) -> BlockShape {
        self.shape
    }

    /// Crop extent in cells this grid was built for.
    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }

    /// Blocks per axis, `ceil(dimension / shape)`. Partial edge blocks exist and hold
    /// set bits only for cells inside the crop.
    pub fn block_dims(&self) -> [u32; 3] {
        self.block_dims
    }

    /// Number of blocks in the grid.
    pub fn blocks(&self) -> usize {
        block_count(self.block_dims).unwrap_or(0)
    }

    /// Packed words, `blocks * words_per_block` entries.
    pub fn words(&self) -> &[u32] {
        &self.words
    }

    /// Occupancy payload bytes, excluding `Vec` metadata and allocator slack.
    pub fn bytes(&self) -> usize {
        self.words.len() * size_of::<u32>()
    }

    /// Number of solid bits set; equals the solid cell count of the source materials.
    pub fn bits_set(&self) -> usize {
        self.bits_set
    }

    /// Index of an in-range block, `x + bx * (y + by * z)`.
    pub fn block_index(&self, block: [u32; 3]) -> Option<usize> {
        block_index(block, self.block_dims)
    }

    /// Words of one block, or `None` when the block index is out of range.
    pub fn block_words(&self, block_index: usize) -> Option<&[u32]> {
        let per_block = self.shape.words_per_block();
        let start = block_index.checked_mul(per_block)?;
        self.words.get(start..start + per_block)
    }

    /// Whether any cell of the block is solid. Out-of-range indices read as empty.
    pub fn block_occupied(&self, block_index: usize) -> bool {
        self.block_words(block_index)
            .is_some_and(|words| words.iter().any(|&word| word != 0))
    }

    /// Whether one local cell is solid. Out-of-range coordinates read as empty.
    pub fn cell_occupied(&self, block_index: usize, local: [u32; 3]) -> bool {
        let dims = self.shape.dims();
        if local[0] >= dims[0] || local[1] >= dims[1] || local[2] >= dims[2] {
            return false;
        }
        let bit = self.shape.bit_index(local);
        self.block_words(block_index)
            .and_then(|words| words.get(bit / 32))
            .is_some_and(|word| word & (1u32 << (bit % 32)) != 0)
    }

    /// Sets one cell's bit and reports the bounded write it caused.
    ///
    /// `None` means the cell lies outside the crop. An unchanged request writes nothing.
    pub fn set_cell(&mut self, cell: [u32; 3], solid: bool) -> Option<BitUpdate> {
        let dims = self.shape.dims();
        if cell[0] >= self.dimensions[0]
            || cell[1] >= self.dimensions[1]
            || cell[2] >= self.dimensions[2]
        {
            return None;
        }
        let block = [cell[0] / dims[0], cell[1] / dims[1], cell[2] / dims[2]];
        let local = [cell[0] % dims[0], cell[1] % dims[1], cell[2] % dims[2]];
        let block_index = block_index(block, self.block_dims)?;
        let empty_before = !self.block_occupied(block_index);
        let bit = self.shape.bit_index(local);
        let word = block_index * self.shape.words_per_block() + bit / 32;
        let mask = 1u32 << (bit % 32);
        let stored = *self.words.get(word)?;
        let changed = (stored & mask != 0) != solid;
        if changed {
            let next = if solid { stored | mask } else { stored & !mask };
            if let Some(slot) = self.words.get_mut(word) {
                *slot = next;
            }
            self.bits_set = if solid {
                self.bits_set + 1
            } else {
                self.bits_set.saturating_sub(1)
            };
        }
        Some(BitUpdate {
            changed,
            word,
            block_empty_before: empty_before,
            block_empty_after: !self.block_occupied(block_index),
        })
    }
}

fn block_count(block_dims: [u32; 3]) -> Result<usize, HierarchyError> {
    block_dims
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d as usize))
        .ok_or(HierarchyError::VolumeCells)
}

fn block_index(block: [u32; 3], block_dims: [u32; 3]) -> Option<usize> {
    let [x, y, z] = block;
    let [dx, dy, _] = block_dims;
    if x >= dx || y >= dy || z >= block_dims[2] {
        return None;
    }
    Some(x as usize + dx as usize * (y as usize + dy as usize * z as usize))
}
