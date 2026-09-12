//! Bounded hierarchical voxel ray traversal experiment with packed occupancy (issue 42).
//!
//! This crate is an **experiment**, not a selected production renderer path. It answers
//! one question: can a bounded crop of authoritative voxel material be traversed
//! correctly through a compact block-occupancy structure, with the same hits, materials,
//! normals and depth as the retained dense reference? See
//! [the log](../../../docs/performance/logs/ray-hierarchy-experiment.md) for the measured
//! comparison, provenance of the referenced prior art, and the integration handoff.
//!
//! # Relation to the retained reference
//!
//! The reference lineage is
//! [`matterweave_render::ray_reference`] (bounded crop, fragment shader) anchored to
//! [`matterweave_core::World::raycast`] (whole-world CPU DDA). This crate adds no second
//! renderer: it derives from an existing reference pack, keeps the reference traversal as
//! a mode, and adds packed-occupancy modes beside it.
//!
//! Semantics inherited from the reference, shared by every mode through one entry routine
//! and one fine-step routine:
//!
//! - half-open crop `[origin, origin + dimensions)`, `MAX_CELLS`/`MAX_AXIS` bounds and
//!   `+/-8192` coordinate bound from `RayVolume::pack`;
//! - slab clipping that branches on zero direction components instead of dividing, with a
//!   parallel ray on the upper face outside the box and on the lower face inside;
//! - zero-length AABB overlap rejected before any cell is inspected;
//! - external entries snap only known slab planes, then cross all exactly tied internal
//!   grid planes together; an origin already inside inspects `floor(origin)` first;
//! - tied axes step simultaneously and the lowest crossed axis supplies the normal;
//! - crossing distances are recomputed from integer planes every step in `f64`, which is
//!   the host analogue of the shader's f32 recomputation;
//! - outer iteration cap `dims.x + dims.y + dims.z + 1`;
//! - inside-solid starts report distance 0 and a zero normal.
//!
//! Documented deviations, all deliberate:
//!
//! - The shader normalizes an unprojection segment to get its direction. Here the caller
//!   supplies a unit direction and the clipped segment length, so the entry, tie and step
//!   rules can be compared bit-for-bit between modes.
//! - `World::raycast` accumulates `next += delta` in `f64` from the *world* origin and has
//!   no crop; this crate recomputes plane distances and excludes everything outside the
//!   crop. Where the crop is irrelevant, hits agree exactly on cell, material and normal,
//!   and agree on distance up to the accumulated-vs-recomputed difference, which the
//!   differential tests bound. Crop exclusion and the zero-length-overlap rule are the
//!   two intentional differences and both are asserted explicitly.
//! - The reference writes projected depth; here depth is derived from the same hit
//!   distance by [`clip_depth`], the host mirror of that fragment arithmetic.
//!
//! # Models
//!
//! [`TraversalMode::Reference`] is the dense reference lineage: one material word read per
//! visited cell, no occupancy data. [`TraversalMode::BlockMask`] runs the same per-cell
//! DDA but consults packed block occupancy for memory access: an empty block costs one
//! word load and no material reads, and a set bit gates each material read.
//! [`TraversalMode::BlockStep`] adds a coarse step over empty blocks with an exact
//! fine-plane catch-up.
//!
//! Occupancy is one bit per cell in fixed blocks ([`BlockShape`], at most 512 bits), kept
//! in `u32` words beside, never instead of, the dense material words. Authoritative voxels
//! stay in `World`; [`HierarchyVolume`] is a derived snapshot with a provenance key, and
//! [`HierarchyVolume::patch_cell`] is a bounded in-memory patch that marks the snapshot
//! stale for its source world.
//!
//! # Limits
//!
//! CPU only. No GPU pipeline, no Android execution, no measured performance claim, and no
//! multi-level tree or DAG: one block level over a fixed bounded crop. Counters in
//! [`TraversalStats`] are buffer-access counts that describe structure, not timings.

mod occupancy;
mod traverse;
mod volume;

pub use matterweave_core::RayHit;
pub use occupancy::{BitUpdate, BlockShape, OccupancyGrid, MAX_BLOCK_BITS};
pub use traverse::{TraversalMode, TraversalStats};
pub use volume::{
    cells_in, clip_depth, BuildReport, HierarchyError, HierarchyVolume, MemoryStats, PatchReport,
    SourceKey, MAX_COORD,
};

#[cfg(test)]
mod differential_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod occupancy_tests;
#[cfg(test)]
mod traversal_tests;
