//! Traversal over a bounded snapshot: the dense reference lineage and the packed
//! occupancy candidates.
//!
//! Semantics are pinned to the retained reference: the crop's half-open AABB, slab
//! entry rules, simultaneous stepping of tied axes, lowest-axis normals, f64 distances
//! recomputed from integer planes and the outer iteration cap
//! `dims.x + dims.y + dims.z + 1`. See the crate documentation for the exact list of
//! inherited and deviating behaviors.

use crate::occupancy::{BlockShape, OccupancyGrid, MAX_BLOCK_BITS};
use crate::{HierarchyError, HierarchyVolume};
use matterweave_core::RayHit;

/// Which traversal a snapshot runs. All modes share one entry routine and one fine-step
/// routine, so hits can only differ where the mode changes memory access, not geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraversalMode {
    /// Dense per-cell DDA over the crop's material words, reading one material word per
    /// visited cell: the reference lineage this experiment is compared against.
    Reference,
    /// Per-cell DDA where packed block occupancy decides memory access: one block fetch
    /// per block entry (all of its words), then no material reads inside an empty block
    /// and one gated material read per cell whose bit is set.
    BlockMask,
    /// [`TraversalMode::BlockMask`] plus a coarse step over empty blocks: one outer
    /// iteration replaces the fine steps through an empty block, paid for with an exact
    /// fine-plane catch-up.
    BlockStep,
}

/// Structural traversal counters. They are buffer-access counts, not timings and not
/// measured bandwidth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TraversalStats {
    /// Outer loop iterations, including the iteration that resolves the ray.
    pub iterations: u64,
    /// Fine single-cell steps.
    pub fine_steps: u64,
    /// Coarse steps that left an empty block.
    pub block_steps: u64,
    /// Fine-plane evaluations performed by the coarse-step catch-up.
    pub catch_up_planes: u64,
    /// Occupancy inspections at block granularity.
    pub blocks_checked: u64,
    /// Occupancy words fetched into the block cache: `words_per_block` per block entry.
    pub occupancy_word_loads: u64,
    /// Cells inspected individually, by bit test or material read.
    pub cells_examined: u64,
    /// Material word reads.
    pub material_reads: u64,
    /// Whether the iteration cap was reached without resolving the ray.
    pub exhausted: bool,
}

impl HierarchyVolume {
    /// Traverses without returning counters.
    ///
    /// An exhausted iteration cap and a crop miss both report `None`; use
    /// [`Self::trace_stats`] when a caller must distinguish them.
    pub fn trace(
        &self,
        mode: TraversalMode,
        origin: [f64; 3],
        direction: [f64; 3],
        max_distance: f64,
    ) -> Result<Option<RayHit>, HierarchyError> {
        self.trace_stats(mode, origin, direction, max_distance)
            .map(|(hit, _)| hit)
    }

    /// Traverses and returns the structural counters of that traversal.
    ///
    /// `origin` and `direction` are world space. `direction` must be finite and unit
    /// length within 1e-6; the reference derives it by normalizing an unprojection
    /// segment, so a caller with a segment normalizes it the same way before calling.
    /// `max_distance` is the clipped segment length: it must be finite and non-negative,
    /// a finite value above `MAX_RAY_DISTANCE` is clamped like `World::raycast`, and `0`
    /// answers the inside-solid query with the distance-0 hit. A miss decided by the
    /// crop clip returns zeroed counters.
    pub fn trace_stats(
        &self,
        mode: TraversalMode,
        origin: [f64; 3],
        direction: [f64; 3],
        max_distance: f64,
    ) -> Result<(Option<RayHit>, TraversalStats), HierarchyError> {
        let Some(mut walk) = Walk::new(self, mode, origin, direction, max_distance)? else {
            return Ok((None, TraversalStats::default()));
        };
        let hit = walk.run();
        Ok((hit, walk.stats))
    }

    /// Outer iteration cap, inherited from the reference's documented
    /// `dims.x + dims.y + dims.z + 1`. Every outer iteration of every mode crosses at
    /// least one fine grid plane, so the cap covers fine steps and coarse steps alike.
    pub fn max_iterations(&self) -> u64 {
        self.dimensions().iter().map(|&d| u64::from(d)).sum::<u64>() + 1
    }
}

struct Walk<'a> {
    mode: TraversalMode,
    grid: Option<&'a OccupancyGrid>,
    materials: &'a [u32],
    lower: [i32; 3],
    dims: [u32; 3],
    shape: BlockShape,
    bound: u64,
    origin: [f64; 3],
    direction: [f64; 3],
    step: [i64; 3],
    cell: [i64; 3],
    distance: f64,
    exit: f64,
    normal: [i32; 3],
    sentinel: f64,
    cached_block: Option<usize>,
    /// Words of `cached_block`, fetched once per block entry and reused by every cell
    /// the ray visits inside that block.
    cached_words: [u32; MAX_BLOCK_BITS / 32],
    /// Cached `block_occupied` answer, maintained by [`Self::refill_block`].
    cached_any: bool,
    stats: TraversalStats,
}

impl<'a> Walk<'a> {
    fn new(
        volume: &'a HierarchyVolume,
        mode: TraversalMode,
        origin: [f64; 3],
        direction: [f64; 3],
        max_distance: f64,
    ) -> Result<Option<Self>, HierarchyError> {
        let norm_sq = direction.iter().map(|v| v * v).sum::<f64>();
        if !norm_sq.is_finite() || (norm_sq - 1.0).abs() > 1e-6 {
            return Err(HierarchyError::InvalidDirection { direction });
        }
        if !origin.iter().all(|v| v.is_finite()) {
            return Err(HierarchyError::InvalidRay { max_distance });
        }
        if !max_distance.is_finite() || max_distance < 0.0 {
            return Err(HierarchyError::InvalidRay { max_distance });
        }
        // A finite range above the cap is clamped, as `World::raycast` does; this is
        // also the traversal's own work bound.
        let limit = max_distance.min(f64::from(matterweave_core::MAX_RAY_DISTANCE));
        let lower = volume.origin();
        let dims = volume.dimensions();
        let mut entry = 0.0f64;
        let mut exit = limit;
        let mut slab_near = [-1.0f64; 3];
        for axis in 0..3 {
            let lo = f64::from(lower[axis]);
            let hi = lo + f64::from(dims[axis]);
            if direction[axis] == 0.0 {
                // Parallel rays never divide: on the upper face they are outside the
                // half-open box, on the lower face they are inside.
                if origin[axis] < lo || origin[axis] >= hi {
                    return Ok(None);
                }
            } else {
                let a = (lo - origin[axis]) / direction[axis];
                let b = (hi - origin[axis]) / direction[axis];
                slab_near[axis] = a.min(b);
                entry = entry.max(a.min(b));
                exit = exit.min(a.max(b));
            }
        }
        let step = direction.map(|v| {
            if v > 0.0 {
                1
            } else if v < 0.0 {
                -1
            } else {
                0
            }
        });
        let starts_inside = (0..3).all(|axis| {
            origin[axis] >= f64::from(lower[axis])
                && origin[axis] < f64::from(lower[axis]) + f64::from(dims[axis])
        });
        // Zero-length overlap is not geometry, including corner-only contact. The one
        // exception is the caller's explicit zero-length query (`max_distance == 0`)
        // from inside the crop: `World::raycast` answers it with the origin cell at
        // distance 0 and a zero normal, and this crate follows the oracle there. The
        // reference shader discards a zero-length segment and cannot express the query.
        if entry >= exit && !(max_distance == 0.0 && starts_inside) {
            return Ok(None);
        }
        // Snap only known slab-entry planes, never bias the whole ray.
        let mut local = [0.0f64; 3];
        for axis in 0..3 {
            local[axis] = origin[axis] + direction[axis] * entry - f64::from(lower[axis]);
            if !starts_inside && slab_near[axis] == entry {
                local[axis] = if step[axis] > 0 {
                    0.0
                } else {
                    f64::from(dims[axis])
                };
            }
        }
        let mut cell = [
            local[0].floor() as i64,
            local[1].floor() as i64,
            local[2].floor() as i64,
        ];
        let mut normal = [0i32; 3];
        let mut first = true;
        for axis in 0..3 {
            // An external entry can tie an internal grid plane on another axis: cross
            // all of those together before inspecting the first inside cell. An origin
            // already inside instead checks floor(origin) first, like `World::raycast`.
            if !starts_inside && step[axis] != 0 && local[axis] == local[axis].floor() {
                if step[axis] < 0 {
                    cell[axis] -= 1;
                }
                if first {
                    normal[axis] = -step[axis] as i32;
                    first = false;
                }
            }
        }
        Ok(Some(Self {
            mode,
            grid: (mode != TraversalMode::Reference).then(|| volume.occupancy()),
            materials: volume.materials(),
            lower,
            dims,
            shape: volume.occupancy().shape(),
            bound: volume.max_iterations(),
            origin,
            direction,
            step,
            cell,
            distance: entry,
            exit,
            normal,
            sentinel: limit + 1.0,
            cached_block: None,
            cached_words: [0; MAX_BLOCK_BITS / 32],
            cached_any: false,
            stats: TraversalStats::default(),
        }))
    }

    fn run(&mut self) -> Option<RayHit> {
        let bound = self.bound;
        while self.stats.iterations < bound {
            self.stats.iterations += 1;
            if (0..3)
                .any(|axis| self.cell[axis] < 0 || self.cell[axis] >= i64::from(self.dims[axis]))
            {
                return None;
            }
            let cell = [
                self.cell[0] as u32,
                self.cell[1] as u32,
                self.cell[2] as u32,
            ];
            if let Some(grid) = self.grid {
                let [sx, sy, sz] = self.shape.dims();
                let block = [cell[0] / sx, cell[1] / sy, cell[2] / sz];
                let Some(block_index) = grid.block_index(block) else {
                    // An in-range cell always maps to an in-range block. Keep the miss
                    // in release builds and fail loudly under test.
                    debug_assert!(false, "cell {cell:?} maps outside the occupancy grid");
                    return None;
                };
                self.stats.blocks_checked += 1;
                if self.cached_block != Some(block_index) {
                    self.refill_block(grid, block_index);
                }
                if self.cached_any {
                    let local = [cell[0] % sx, cell[1] % sy, cell[2] % sz];
                    self.stats.cells_examined += 1;
                    if self.cell_occupied(local) {
                        self.stats.material_reads += 1;
                        let material = self.material_word(cell);
                        if material != 0 {
                            return Some(self.hit(cell, material));
                        }
                    }
                } else if self.mode == TraversalMode::BlockStep {
                    if !self.skip_empty_block(grid, block_index) {
                        return None;
                    }
                    continue;
                }
                if !self.advance() {
                    return None;
                }
                continue;
            }
            self.stats.cells_examined += 1;
            self.stats.material_reads += 1;
            let material = self.material_word(cell);
            if material != 0 {
                return Some(self.hit(cell, material));
            }
            if !self.advance() {
                return None;
            }
        }
        // Unreachable for a ray inside the crop; a structural guard against an
        // unbounded loop rather than a silent hang.
        self.stats.exhausted = true;
        None
    }

    /// Fetches the words of one block into the cache.
    ///
    /// This is the only occupancy word read a traversal performs: `words_per_block`
    /// words per block entry, reused by every cell the ray visits inside that block.
    fn refill_block(&mut self, grid: &OccupancyGrid, block_index: usize) {
        let words = grid
            .block_words(block_index)
            .expect("block index came from this grid");
        self.cached_words[..words.len()].copy_from_slice(words);
        self.cached_any = words.iter().any(|&word| word != 0);
        self.stats.occupancy_word_loads += words.len() as u64;
        self.cached_block = Some(block_index);
    }

    /// Bit test against the cached block words; `local` is in block coordinates and
    /// inside the shape, so the bit always addresses a cached word.
    fn cell_occupied(&self, local: [u32; 3]) -> bool {
        debug_assert!(self.cached_block.is_some());
        let bit = self.shape.bit_index(local);
        self.cached_words[bit / 32] & (1u32 << (bit % 32)) != 0
    }

    /// Fine step: recompute every crossing from integer planes instead of accumulating.
    /// Ties step together and the lowest crossed axis supplies the normal.
    fn advance(&mut self) -> bool {
        // A zero step keeps the finite sentinel: the reference never divides by zero.
        let mut next = [self.sentinel; 3];
        for (axis, slot) in next.iter_mut().enumerate() {
            if self.step[axis] == 0 {
                continue;
            }
            let boundary = self.crossing_boundary(axis);
            *slot = (boundary as f64 - self.origin[axis]) / self.direction[axis];
        }
        let distance = next[0].min(next[1].min(next[2]));
        if distance > self.exit {
            return false;
        }
        self.distance = distance;
        self.normal = [0; 3];
        let mut first = true;
        for (axis, &crossing) in next.iter().enumerate() {
            if crossing == distance && self.step[axis] != 0 {
                self.cell[axis] += self.step[axis];
                if first {
                    self.normal[axis] = -self.step[axis] as i32;
                    first = false;
                }
            }
        }
        self.stats.fine_steps += 1;
        true
    }

    /// One coarse step through the empty block the ray is currently in.
    ///
    /// The block's exit plane is crossed, and every axis is caught up through the fine
    /// planes it crosses strictly before that exit using the same `f64` plane formula as
    /// [`Self::advance`]. Axes whose plane lands exactly on the exit keep the reference's
    /// simultaneous tie rule: a block exit jumps to the new block's entry cell, while a
    /// colliding interior plane steps one cell and can supply the normal.
    fn skip_empty_block(&mut self, grid: &OccupancyGrid, block_index: usize) -> bool {
        debug_assert!(!self.cached_any);
        debug_assert!(
            !grid.block_occupied(block_index),
            "the cached block words agree with the grid"
        );
        let shape = self.shape.dims().map(i64::from);
        let mut crossed = [0i64; 3];
        let mut entry_cell = [0i64; 3];
        let mut plane_t = [self.sentinel; 3];
        for axis in 0..3 {
            if self.step[axis] == 0 {
                continue;
            }
            let base = self.cell[axis].div_euclid(shape[axis]);
            let (plane_cell, after) = if self.step[axis] > 0 {
                (base + 1, (base + 1) * shape[axis])
            } else {
                (base, (base - 1) * shape[axis] + shape[axis] - 1)
            };
            crossed[axis] = i64::from(self.lower[axis]) + plane_cell * shape[axis];
            entry_cell[axis] = after;
            plane_t[axis] = (crossed[axis] as f64 - self.origin[axis]) / self.direction[axis];
        }
        let distance = plane_t[0].min(plane_t[1].min(plane_t[2]));
        if distance > self.exit {
            return false;
        }
        for axis in 0..3 {
            if self.step[axis] == 0 {
                continue;
            }
            while (self.crossing_boundary(axis) as f64 - self.origin[axis]) / self.direction[axis]
                < distance
            {
                self.cell[axis] += self.step[axis];
                self.stats.catch_up_planes += 1;
            }
        }
        self.distance = distance;
        self.normal = [0; 3];
        let mut first = true;
        for axis in 0..3 {
            if self.step[axis] == 0 {
                continue;
            }
            let boundary = self.crossing_boundary(axis);
            if (boundary as f64 - self.origin[axis]) / self.direction[axis] != distance {
                continue;
            }
            if boundary == crossed[axis] {
                self.cell[axis] = entry_cell[axis];
            } else {
                self.cell[axis] += self.step[axis];
            }
            if first {
                self.normal[axis] = -self.step[axis] as i32;
                first = false;
            }
        }
        self.stats.block_steps += 1;
        true
    }

    /// Integer plane the cell crosses next along `axis` in the step direction.
    fn crossing_boundary(&self, axis: usize) -> i64 {
        i64::from(self.lower[axis]) + self.cell[axis] + i64::from(self.step[axis] > 0)
    }

    fn material_word(&self, cell: [u32; 3]) -> u32 {
        let [dx, dy, _] = self.dims;
        let index = (cell[0] + dx * (cell[1] + dy * cell[2])) as usize;
        debug_assert!(index < self.materials.len());
        self.materials[index]
    }

    /// World-space hit: `RayHit` cells are world cells, like `World::raycast` reports.
    fn hit(&self, cell: [u32; 3], material: u32) -> RayHit {
        // Construction rejects material words above `u8::MAX` and patching takes `u8`.
        debug_assert!(material <= u32::from(u8::MAX));
        RayHit {
            cell: [
                self.lower[0] + cell[0] as i32,
                self.lower[1] + cell[1] as i32,
                self.lower[2] + cell[2] as i32,
            ],
            normal: self.normal,
            distance: self.distance as f32,
            material: material as u8,
        }
    }
}
