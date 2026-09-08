//! Deterministic bounded snapshot of a detail volume.
//!
//! Cells are stored as X-axis runs `[x, y, z, length, material]` in the stable
//! iteration order of `DetailVolume::iter_cells` (chunk-major, then z, y, x), so
//! equal content always produces equal bytes. Run count is bounded by
//! [`MAX_SNAPSHOT_RUNS`]; there is no unbounded per-cell text blob.
//!
//! Loading is validated in two phases: every run is range-, budget- and
//! overlap-checked *before* any voxel payload is allocated or written.
//! Decoding a [`VolumeSnapshot`] struct from a caller-chosen source (raw serde,
//! a network frame, another format) is the caller's responsibility to bound;
//! [`DetailVolume::from_json_bytes`] is provided for the common JSON case and
//! enforces [`MAX_SNAPSHOT_JSON_BYTES`] before serde allocates anything.

use crate::{
    DetailError, DetailVolume, Result, Scale, MAX_CELL_COORD, MAX_VOLUME_CELLS, MAX_VOLUME_CHUNKS,
};
use matterweave_core::CHUNK_EDGE;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SNAPSHOT_VERSION: u32 = 1;
/// Upper bound on runs in one snapshot.
pub const MAX_SNAPSHOT_RUNS: usize = 2_000_000;
/// Byte cap applied before JSON is parsed into a snapshot.
pub const MAX_SNAPSHOT_JSON_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VolumeSnapshot {
    pub version: u32,
    pub id: String,
    pub scale_m: f32,
    /// Source revision at capture time. Restoring rebuilds content, not history:
    /// the restored volume starts its own revision sequence.
    pub source_revision: u64,
    pub occupied_cells: usize,
    /// `[x, y, z, length, material]`, non-overlapping.
    pub runs: Vec<[i32; 5]>,
}

impl DetailVolume {
    pub fn snapshot(&self) -> VolumeSnapshot {
        let mut runs: Vec<[i32; 5]> = Vec::new();
        for (cell, material) in self.iter_cells() {
            match runs.last_mut() {
                Some(run)
                    if run[1] == cell[1]
                        && run[2] == cell[2]
                        && run[4] == i32::from(material)
                        && run[0] + run[3] == cell[0] =>
                {
                    run[3] += 1;
                }
                _ => runs.push([cell[0], cell[1], cell[2], 1, i32::from(material)]),
            }
        }
        VolumeSnapshot {
            version: SNAPSHOT_VERSION,
            id: self.id().to_string(),
            scale_m: self.scale().metres(),
            source_revision: self.revision(),
            occupied_cells: self.occupied_cells(),
            runs,
        }
    }

    /// Bounded JSON loader: rejects oversized input before handing bytes to serde.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_SNAPSHOT_JSON_BYTES {
            return Err(DetailError::MalformedSnapshot("json byte cap exceeded"));
        }
        let snapshot: VolumeSnapshot = serde_json::from_slice(bytes)
            .map_err(|_| DetailError::MalformedSnapshot("json decode failed"))?;
        Self::from_snapshot(&snapshot)
    }

    /// Validates the whole snapshot, then writes it. Nothing is allocated for the
    /// voxel payload until every range, budget and overlap check has passed.
    ///
    /// Runs are stored chunk-major, so ordering is *not* required to be globally
    /// ascending in x/y/z; overlap is detected on a sorted copy instead, and is
    /// rejected independently of the declared `occupied_cells`.
    pub fn from_snapshot(snapshot: &VolumeSnapshot) -> Result<Self> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(DetailError::MalformedSnapshot("unsupported version"));
        }
        if snapshot.runs.len() > MAX_SNAPSHOT_RUNS {
            return Err(DetailError::MalformedSnapshot("run count over bound"));
        }
        let scale = Scale::new(snapshot.scale_m)?;

        // Phase 1: validate every run and accumulate budgets.
        let mut total: usize = 0;
        let mut spans: Vec<(i32, i32, i32, i32)> = Vec::with_capacity(snapshot.runs.len());
        let mut chunks: BTreeSet<[i32; 3]> = BTreeSet::new();
        for &[x, y, z, length, material] in &snapshot.runs {
            if length <= 0 {
                return Err(DetailError::MalformedSnapshot("invalid run length"));
            }
            if !(1..=255).contains(&material) {
                return Err(DetailError::MalformedSnapshot("invalid material"));
            }
            let end = x
                .checked_add(length)
                .ok_or(DetailError::MalformedSnapshot("run overflows axis"))?;
            let in_range = |v: i32| (-MAX_CELL_COORD..=MAX_CELL_COORD).contains(&v);
            if !in_range(x) || !in_range(end - 1) || !in_range(y) || !in_range(z) {
                return Err(DetailError::MalformedSnapshot("run outside cell range"));
            }
            total = total
                .checked_add(length as usize)
                .ok_or(DetailError::MalformedSnapshot("cell total overflows"))?;
            if total > MAX_VOLUME_CELLS {
                return Err(DetailError::BudgetExceeded("volume cell budget"));
            }
            for cx in x.div_euclid(CHUNK_EDGE)..=(end - 1).div_euclid(CHUNK_EDGE) {
                chunks.insert([cx, y.div_euclid(CHUNK_EDGE), z.div_euclid(CHUNK_EDGE)]);
                if chunks.len() > MAX_VOLUME_CHUNKS {
                    return Err(DetailError::BudgetExceeded("volume chunk budget"));
                }
            }
            spans.push((y, z, x, end));
        }
        // Overlap is a structural error whatever the declared count says.
        spans.sort_unstable();
        for pair in spans.windows(2) {
            let (ay, az, _, aend) = pair[0];
            let (by, bz, bx, _) = pair[1];
            if ay == by && az == bz && bx < aend {
                return Err(DetailError::MalformedSnapshot("overlapping runs"));
            }
        }
        if total != snapshot.occupied_cells {
            return Err(DetailError::MalformedSnapshot(
                "occupied cell count mismatch",
            ));
        }

        // Phase 2: write validated content.
        let mut volume = DetailVolume::new(snapshot.id.clone(), scale);
        for &[x, y, z, length, material] in &snapshot.runs {
            for cx in x..x + length {
                volume.set([cx, y, z], material as u8)?;
            }
        }
        debug_assert_eq!(volume.occupied_cells(), snapshot.occupied_cells);
        Ok(volume)
    }
}
