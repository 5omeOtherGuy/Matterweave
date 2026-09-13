//! Bounded exact dependency tracking for retained mesh-proxy edits.
//!
//! `IndirectVolume` publishes only a complete volume for one key, so when a
//! mesh proxy moves or is edited the whole volume is normally recomputed. That
//! is correct but wastes work: a one-cell body move cannot change most face
//! values. This module records, per completed face, the proxy cells the face's
//! value actually depends on, and answers the one question the retention path
//! needs: which completed faces does an old/new proxy diff invalidate?
//!
//! # What is recorded, and why it is exact
//!
//! A face value is a deterministic function of the world (constant for one
//! revision/epoch key), the sun, the quadrature, the face's own cell and its
//! outward neighbor (the exposure test), and the *segments* the sample loop
//! traces: a gather segment of up to `distance` from the face origin, then a
//! hit-to-sun segment of up to `distance` from the gather hit point. Both
//! segments are `trace_scene`, so they read `World` cells and proxy cells.
//!
//! World cells are constant inside one key: any world revision or replacement
//! epoch change clears the whole volume in `IndirectVolume::update`. Only proxy
//! cells can change without a key change, so only proxy cells are recorded
//! here. For each traced segment [`walk_segment`] visits exactly the cells
//! `matterweave_core::World::raycast` reads - the DDA path is purely geometric
//! until it stops, so the cells read are the from-origin cells up to and
//! including the first solid one, or every cell up to the traversal limit when
//! nothing is hit. Empty cells on the path are recorded too: a later occluder
//! inserted on a previously empty ray must invalidate the face, and a removed
//! first hit must also invalidate it (the cells behind that hit were never
//! read and are therefore not recorded, but the removed hit cell itself is).
//!
//! Exactness follows: if none of the recorded cells changed, every recorded
//! cell holds the same material, so the DDA reads the same cells, stops at the
//! same hit and the sample values are bit-identical. If a cell that is not
//! recorded changed, it was never read in the completed state, and reading it
//! requires traversing a recorded cell first - which is invalidated.
//!
//! The exposure test also reads the face's own cell and its outward neighbor;
//! both are recorded with the ray cells when a face starts sampling, and the
//! edit path additionally expands every changed cell over its closed
//! neighborhood independently of the bitsets, so a face whose exposure changed
//! is always invalidated even if a bitset is missing or partial.
//!
//! # Bounds
//!
//! The bitset index space is the volume's own coverage box (`cells <=
//! MAX_FACE_SLOTS / 6`), so one face costs `ceil(cells / 64) * 8` bytes: at
//! most 512 bytes. Bitsets are allocated lazily, once per face, from an arena
//! capped by the caller's `max_bytes`. When the cap is reached (or the arena
//! cannot grow), that face is marked [`UNTRACKED`]: it keeps a correct value
//! but is recomputed on every proxy edit instead of being retained. The status
//! reports tracked, untracked and outside-cell counts so the cost and the
//! degradation are visible, never silent.
//!
//! A changed cell outside the index space cannot be represented, so such an
//! edit is a coverage change: [`invalidate_edit`] reports `tracked: false` and
//! the caller clears every value, the same conservative behavior as replacing
//! the representation. Recording a cell outside the index space is ignored for
//! the same reason: under the coverage rule it holds air in every attached
//! proxy, so it cannot change without triggering that fallback.

use crate::indirect::MeshProxy;
use matterweave_core::MAX_RAY_DISTANCE;
use std::cmp::Ordering;

/// The face has no dependency bitset yet (never sampled) or had one and lost it
/// to a coverage fallback. Faces without a bitset are never retained on their
/// bits, but only faces that hold a completed value must be invalidated.
const NO_BITS: u32 = u32::MAX;
/// The face's bitset could not be allocated within the tracking memory cap. It
/// is recomputed on every proxy edit.
const UNTRACKED: u32 = u32::MAX - 1;

/// Per-face proxy-cell dependency bitsets for one coverage box.
pub(crate) struct MeshDependencies {
    origin: [i32; 3],
    dimensions: [u32; 3],
    /// Cells in the index space.
    cells: usize,
    /// `u64` words per face bitset.
    words: usize,
    /// Face slots (`cells * 6`).
    slots: usize,
    /// Arena budget in bytes; the offsets array and the changed mask are already
    /// deducted, so this bounds the bitsets themselves.
    arena_cap_bytes: usize,
    /// Arena word offset per face slot, or [`NO_BITS`] / [`UNTRACKED`].
    offsets: Vec<u32>,
    /// Face-major packed bitsets.
    arena: Vec<u64>,
    /// Scratch mask of cells whose proxy material differs in the current edit.
    changed: Vec<u64>,
    tracked_faces: usize,
    untracked_faces: usize,
    /// Recorded segment cells outside the index space, reported for honesty.
    outside_cells: usize,
    /// Face slot currently recording, set by [`Self::begin_face`].
    active: Option<usize>,
    /// The active face's bitset must be zeroed before the next bit is set.
    clear_active: bool,
}

impl MeshDependencies {
    /// Validates the index space and allocates the per-face offset and changed
    /// mask arrays. `cap_bytes` is clamped to the engine maximum by the caller
    /// and must cover those arrays plus at least one face bitset.
    pub(crate) fn new(
        origin: [i32; 3],
        dimensions: [u32; 3],
        slots: usize,
        cap_bytes: usize,
    ) -> Result<Self, String> {
        let cells = dimensions
            .iter()
            .try_fold(1usize, |n, &d| n.checked_mul(d as usize))
            .filter(|&n| n > 0 && n * 6 == slots)
            .ok_or("Dependency tracking requires one face slot per coverage cell face")?;
        let words = cells.div_ceil(64);
        let offsets_bytes = slots
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or("Dependency tracking offsets overflow")?;
        let changed_bytes = words
            .checked_mul(std::mem::size_of::<u64>())
            .ok_or("Dependency tracking mask overflow")?;
        let per_face_bytes = words * std::mem::size_of::<u64>();
        let fixed = offsets_bytes
            .checked_add(changed_bytes)
            .ok_or("Dependency tracking size overflow")?;
        let arena_cap_bytes = cap_bytes
            .checked_sub(fixed)
            .filter(|&free| free >= per_face_bytes)
            .ok_or_else(|| {
                format!(
                    "Dependency tracking needs at least {} bytes for {slots} faces of {cells} cells",
                    fixed + per_face_bytes
                )
            })?;
        let mut offsets = Vec::new();
        offsets
            .try_reserve_exact(slots)
            .map_err(|e| format!("Dependency tracking offsets allocation: {e}"))?;
        offsets.resize(slots, NO_BITS);
        let mut changed = Vec::new();
        changed
            .try_reserve_exact(words)
            .map_err(|e| format!("Dependency tracking mask allocation: {e}"))?;
        changed.resize(words, 0);
        Ok(Self {
            origin,
            dimensions,
            cells,
            words,
            slots,
            arena_cap_bytes,
            offsets,
            arena: Vec::new(),
            changed,
            tracked_faces: 0,
            untracked_faces: 0,
            outside_cells: 0,
            active: None,
            clear_active: false,
        })
    }

    pub(crate) fn resident_bytes(&self) -> usize {
        self.offsets.len() * std::mem::size_of::<u32>()
            + self.arena.len() * std::mem::size_of::<u64>()
            + self.changed.len() * std::mem::size_of::<u64>()
            + std::mem::size_of::<Self>()
    }

    pub(crate) fn bits_per_face(&self) -> usize {
        self.cells
    }

    pub(crate) fn face_slots(&self) -> usize {
        self.slots
    }

    pub(crate) fn cap_bytes(&self) -> usize {
        self.arena_cap_bytes
            + self.offsets.len() * std::mem::size_of::<u32>()
            + self.changed.len() * std::mem::size_of::<u64>()
    }

    pub(crate) fn tracked_faces(&self) -> usize {
        self.tracked_faces
    }

    pub(crate) fn untracked_faces(&self) -> usize {
        self.untracked_faces
    }

    pub(crate) fn outside_cells(&self) -> usize {
        self.outside_cells
    }

    /// Start recording the dependency of `slot`. The bitset is zeroed lazily, at
    /// the first recorded cell, so a face that records nothing costs no arena.
    pub(crate) fn begin_face(&mut self, slot: usize) {
        if slot < self.slots {
            self.active = Some(slot);
            self.clear_active = true;
        }
    }

    /// Forget a face's recorded cells without starting a sample loop. A face
    /// whose exposure test just failed traces no ray, so its only dependencies
    /// are its own cell and its neighbor - recorded again the next time it
    /// samples - and stale bits from an older placement must not keep
    /// invalidating it.
    pub(crate) fn reset_face(&mut self, slot: usize) {
        if slot >= self.slots {
            return;
        }
        self.active = None;
        self.clear_active = false;
        if let NO_BITS | UNTRACKED = self.offsets[slot] {
            return;
        }
        let offset = self.offsets[slot] as usize;
        self.arena[offset..offset + self.words].fill(0);
    }

    /// Record that the given world cell was read while computing the active
    /// face. Cells outside the index space are counted and ignored; see the
    /// module docs for why that is exact under the coverage rule.
    pub(crate) fn record_cell(&mut self, cell: [i32; 3]) {
        let Some(index) = self.index_of(cell) else {
            self.outside_cells = self.outside_cells.saturating_add(1);
            return;
        };
        let Some(slot) = self.active else {
            return;
        };
        if self.clear_active {
            self.clear_active = false;
            let offset = match self.offsets[slot] {
                NO_BITS => match self.allocate(slot) {
                    Some(offset) => offset,
                    None => return,
                },
                UNTRACKED => return,
                offset => offset,
            };
            let start = offset as usize;
            self.arena[start..start + self.words].fill(0);
        }
        let offset = match self.offsets[slot] {
            NO_BITS | UNTRACKED => return,
            offset => offset,
        } as usize;
        let word = offset + index / 64;
        self.arena[word] |= 1u64 << (index % 64);
    }

    /// Arena offset for a new bitset, or `None` when the cap or the allocator
    /// refuses. Either way the face becomes [`UNTRACKED`] and is always
    /// recomputed, so a missing bitset can never retain a stale value.
    fn allocate(&mut self, slot: usize) -> Option<u32> {
        let offset = self.arena.len();
        let required = (offset + self.words) * std::mem::size_of::<u64>();
        if required > self.arena_cap_bytes {
            self.offsets[slot] = UNTRACKED;
            self.untracked_faces = self.untracked_faces.saturating_add(1);
            return None;
        }
        if self.arena.try_reserve_exact(self.words).is_err() {
            self.offsets[slot] = UNTRACKED;
            self.untracked_faces = self.untracked_faces.saturating_add(1);
            return None;
        }
        self.arena.resize(offset + self.words, 0);
        let offset = offset as u32;
        self.offsets[slot] = offset;
        self.tracked_faces = self.tracked_faces.saturating_add(1);
        Some(offset)
    }

    /// Face-slot index space offset of a cell, or `None` outside the box.
    pub(crate) fn index_of(&self, cell: [i32; 3]) -> Option<usize> {
        let local: [i64; 3] =
            std::array::from_fn(|a| i64::from(cell[a]) - i64::from(self.origin[a]));
        if (0..3).any(|a| local[a] < 0 || local[a] >= i64::from(self.dimensions[a])) {
            return None;
        }
        let (x, y, z) = (local[0] as usize, local[1] as usize, local[2] as usize);
        Some(x + self.dimensions[0] as usize * (y + self.dimensions[1] as usize * z))
    }

    /// Whether a face's recorded proxy cells intersect the current changed mask.
    /// An [`UNTRACKED`] face always intersects: it must be recomputed.
    fn intersects_changed(&self, slot: usize) -> bool {
        match self.offsets[slot] {
            NO_BITS => false,
            UNTRACKED => true,
            offset => {
                let start = offset as usize;
                self.arena[start..start + self.words]
                    .iter()
                    .zip(&self.changed)
                    .any(|(bits, changed)| bits & changed != 0)
            }
        }
    }
}

/// Counts one proxy edit for reporting and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditCounts {
    /// Cells whose proxy material (including presence) differs.
    pub(crate) changed_cells: usize,
    /// Completed faces whose value was discarded.
    pub(crate) invalidated_faces: usize,
    /// Completed faces whose value survives for the new proxy.
    pub(crate) retained_faces: usize,
    /// Face slots left to resolve for the current key (an exposure test or a
    /// sample loop): the update loop's dirty set.
    pub(crate) dirty_faces: usize,
    /// Lowest dirty slot, so the caller can resume its scan there.
    pub(crate) first_dirty: Option<usize>,
    /// Whether exact dependency tracking decided the invalidation. `false` means
    /// a coverage change: the caller must clear every value.
    pub(crate) tracked: bool,
}

/// Invalidate the completed faces an old/new proxy diff can change.
///
/// The diff is over proxy cells only: the union material changed at a cell whose
/// proxy material changed (the world is constant for one key), and a cell whose
/// proxy material changed under world-solid geometry over-invalidates, which is
/// conservative. Every changed cell is expanded over its closed neighborhood so
/// the exposure tests of those faces are reconsidered, and every tracked face
/// whose recorded segment cells intersect the changed set is invalidated.
pub(crate) fn invalidate_edit(
    deps: &mut MeshDependencies,
    old: Option<&MeshProxy>,
    new: Option<&MeshProxy>,
    values: &mut [[f32; 4]],
    done: &mut [u64],
) -> EditCounts {
    deps.changed.fill(0);
    let mut changed_cells = 0usize;
    let mut outside = false;
    let mut old_cells = old.map(MeshProxy::cells).into_iter().flatten().peekable();
    let mut new_cells = new.map(MeshProxy::cells).into_iter().flatten().peekable();
    loop {
        let (cell, before, after) = match (old_cells.peek().copied(), new_cells.peek().copied()) {
            (None, None) => break,
            (Some((cell, material)), None) => {
                old_cells.next();
                (cell, material, 0)
            }
            (None, Some((cell, material))) => {
                new_cells.next();
                (cell, 0, material)
            }
            (Some((first, first_material)), Some((second, second_material))) => {
                match cell_order(first, second) {
                    Ordering::Less => {
                        old_cells.next();
                        (first, first_material, 0)
                    }
                    Ordering::Greater => {
                        new_cells.next();
                        (second, 0, second_material)
                    }
                    Ordering::Equal => {
                        old_cells.next();
                        new_cells.next();
                        (first, first_material, second_material)
                    }
                }
            }
        };
        if before == after {
            continue;
        }
        changed_cells += 1;
        match deps.index_of(cell) {
            Some(index) => deps.changed[index / 64] |= 1u64 << (index % 64),
            None => outside = true,
        }
    }
    if outside {
        // A changed cell outside the index space cannot be represented exactly.
        // Report the diff and let the caller apply its full-clear semantics.
        return EditCounts {
            changed_cells,
            tracked: false,
            ..EditCounts::default()
        };
    }
    let mut invalidated = 0usize;
    // Independent of the recorded bitsets: every closed neighborhood of a
    // changed cell has its six faces invalidated, so an exposure change is
    // caught even when a face has no bitset or a partial one.
    for index in set_bits(&deps.changed) {
        let Some(cell) = deps.cell_of(index) else {
            continue;
        };
        for neighbor in NEIGHBORHOOD {
            let probe = [
                cell[0] + neighbor[0],
                cell[1] + neighbor[1],
                cell[2] + neighbor[2],
            ];
            let Some(probe_index) = deps.index_of(probe) else {
                continue;
            };
            let base = probe_index * 6;
            for face in 0..6 {
                invalidated += invalidate_slot(values, done, base + face);
            }
        }
    }
    // Exact rays: a completed face whose recorded segment cells intersect the
    // changed set is invalidated; a face with no bitset at all records no ray
    // dependency and is decided by the exposure rule above. `UNTRACKED` faces
    // always intersect and are always recomputed.
    for slot in 0..deps.slots {
        if deps.intersects_changed(slot) {
            invalidated += invalidate_slot(values, done, slot);
        }
    }
    let retained = values.iter().filter(|value| value[3] != 0.0).count();
    let dirty_faces = values.len() - bit_count(done, values.len());
    let first_dirty = (0..values.len()).find(|&slot| !bit_is_set(done, slot));
    EditCounts {
        changed_cells,
        invalidated_faces: invalidated,
        retained_faces: retained,
        dirty_faces,
        first_dirty,
        tracked: true,
    }
}

/// Packed-bit helpers shared with `IndirectVolume`'s per-face `done` bitmap.
pub(crate) fn bit_is_set(bits: &[u64], index: usize) -> bool {
    bits[index / 64] & (1u64 << (index % 64)) != 0
}

pub(crate) fn bit_set(bits: &mut [u64], index: usize) {
    bits[index / 64] |= 1u64 << (index % 64);
}

pub(crate) fn bit_clear(bits: &mut [u64], index: usize) {
    bits[index / 64] &= !(1u64 << (index % 64));
}

/// Number of set bits below `slots`; the tail bits of the last word are ignored.
pub(crate) fn bit_count(bits: &[u64], slots: usize) -> usize {
    let full = slots / 64;
    let mut count = bits[..full]
        .iter()
        .map(|word| word.count_ones() as usize)
        .sum();
    if !slots.is_multiple_of(64) {
        let mask = (1u64 << (slots % 64)) - 1;
        count += (bits[full] & mask).count_ones() as usize;
    }
    count
}

/// Mark one face slot dirty: clear its completed bit (so the update scan
/// revisits it) and zero its value. Returns 1 when it held a value and 0 when it
/// was already dirty, so callers can count idempotently.
fn invalidate_slot(values: &mut [[f32; 4]], done: &mut [u64], slot: usize) -> usize {
    bit_clear(done, slot);
    if values[slot][3] == 0.0 {
        return 0;
    }
    values[slot] = [0.; 4];
    1
}

/// The changed cell and its six axis neighbors.
const NEIGHBORHOOD: [[i32; 3]; 7] = [
    [0, 0, 0],
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

/// Ascending cell order shared by `MeshProxy::cells` (ascending local index) and
/// the index space: `z` major, then `y`, then `x`.
fn cell_order(first: [i32; 3], second: [i32; 3]) -> Ordering {
    (first[2], first[1], first[0]).cmp(&(second[2], second[1], second[0]))
}

/// Indices of the set bits of a packed mask, ascending.
fn set_bits(mask: &[u64]) -> impl Iterator<Item = usize> + '_ {
    mask.iter().enumerate().flat_map(|(word, &bits)| {
        (0..64).filter_map(move |bit| (bits & (1u64 << bit) != 0).then_some(word * 64 + bit))
    })
}

impl MeshDependencies {
    fn cell_of(&self, index: usize) -> Option<[i32; 3]> {
        if index >= self.cells {
            return None;
        }
        let dx = self.dimensions[0] as usize;
        let dy = self.dimensions[1] as usize;
        Some([
            self.origin[0] + (index % dx) as i32,
            self.origin[1] + ((index / dx) % dy) as i32,
            self.origin[2] + (index / (dx * dy)) as i32,
        ])
    }
}

/// Visits exactly the cells `matterweave_core::World::raycast` reads for one
/// segment: the cells of the DDA path from the origin's cell up to and including
/// the first cell with a nonzero material, or every path cell whose entry
/// distance is within `max_distance` when nothing is solid. The traversal path
/// is purely geometric, so the cells read are the same for any material
/// function; `material_at` is called once per visited cell in the same order the
/// DDA calls `World::get`, and it is the authority for where the walk stops.
///
/// This mirrors the shipped DDA instead of duplicating a different traversal
/// because `World` exposes no cell visitor; the mirror is pinned by tests that
/// compare the last visited cell and the reach against `World::raycast` and
/// `MeshProxy::raycast` on the same segments.
pub(crate) fn walk_segment(
    material_at: impl Fn([i32; 3]) -> u8,
    origin: [f32; 3],
    direction: [f32; 3],
    max_distance: f32,
    mut visit: impl FnMut([i32; 3]),
) {
    if !max_distance.is_finite()
        || max_distance < 0.0
        || !origin.iter().chain(direction.iter()).all(|v| v.is_finite())
    {
        return;
    }
    let norm = direction
        .iter()
        .map(|&v| f64::from(v).powi(2))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        return;
    }
    let direction = direction.map(|v| f64::from(v) / norm);
    let origin = origin.map(f64::from);
    if origin
        .iter()
        .any(|v| v.floor() < f64::from(i32::MIN) || v.floor() > f64::from(i32::MAX))
    {
        return;
    }
    let mut cell = origin.map(|v| v.floor() as i32);
    let step = direction.map(|v| {
        if v > 0.0 {
            1
        } else if v < 0.0 {
            -1
        } else {
            0
        }
    });
    let delta = direction.map(|v| {
        if v == 0.0 {
            f64::INFINITY
        } else {
            v.abs().recip()
        }
    });
    let mut next: [f64; 3] = std::array::from_fn(|axis| {
        if step[axis] == 0 {
            return f64::INFINITY;
        }
        let boundary = f64::from(cell[axis]) + if step[axis] > 0 { 1.0 } else { 0.0 };
        (boundary - origin[axis]) / direction[axis]
    });
    let limit = f64::from(max_distance.min(MAX_RAY_DISTANCE));
    for _ in 0..(3 * MAX_RAY_DISTANCE as usize + 4) {
        let solid = material_at(cell) != 0;
        visit(cell);
        if solid {
            return;
        }
        let distance = next.iter().copied().fold(f64::INFINITY, f64::min);
        if distance > limit {
            return;
        }
        for axis in 0..3 {
            if next[axis] == distance {
                let Some(stepped) = cell[axis].checked_add(step[axis]) else {
                    return;
                };
                cell[axis] = stepped;
                next[axis] += delta[axis];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indirect::{
        scene_material, IndirectVolume, MeshGeometry, MeshProxy, ProxyEdit, UpdateBudget,
        MAX_DEPENDENCY_BYTES,
    };
    use crate::static_scene::StaticInstance;
    use crate::Sun;
    use glam::Vec3;
    use matterweave_core::{Mesh, World};

    /// Documented `IndirectVolume` face order: `+X, -X, +Y, -Y, +Z, -Z`.
    const FACE_NORMALS: [[i32; 3]; 6] = [
        [1, 0, 0],
        [-1, 0, 0],
        [0, 1, 0],
        [0, -1, 0],
        [0, 0, 1],
        [0, 0, -1],
    ];

    /// The wetland-shaped coverage box (20 x 10 x 20 cells) and the wetland's
    /// own quadrature, gather distance and per-frame budget. Retention must work
    /// at the production values, so no test lowers them.
    const BOX_ORIGIN: [i32; 3] = [-10, 0, -10];
    const BOX_DIMENSIONS: [u32; 3] = [20, 10, 20];
    const SAMPLES: u32 = 16;
    const GATHER_DISTANCE_M: f32 = 24.0;
    const UPDATE_BUDGET: UpdateBudget = UpdateBudget {
        rays: 1024,
        work: 8192,
    };
    /// Oblique sun: the hit-to-sun segment must have horizontal reach for the
    /// second dependency hop to be exercised at all.
    const SUN: Sun = Sun {
        direction_to_sun: [0.0, 1.0, 5.0],
        intensity: 1.0,
    };
    const SUN_OTHER: Sun = Sun {
        direction_to_sun: [5.0, 1.0, 0.0],
        intensity: 1.0,
    };
    const FLOOR_MATERIAL: u8 = 11;
    const ROCK_MATERIAL: u8 = 12;
    const BODY_MATERIAL: u8 = 200;
    const ALT_MATERIAL: u8 = 201;
    /// A tracking cap generous enough to track every sampled face of these
    /// fixtures; the cap behavior has its own test.
    const TRACKING_CAP: usize = 8 * 1024 * 1024;

    fn palette() -> [[f32; 3]; 256] {
        let mut palette = [[0.5; 3]; 256];
        palette[FLOOR_MATERIAL as usize] = [0.9, 0.05, 0.02];
        palette[ROCK_MATERIAL as usize] = [0.4, 0.5, 0.4];
        palette[BODY_MATERIAL as usize] = [0.8, 0.2, 0.1];
        palette[ALT_MATERIAL as usize] = [0.05, 0.8, 0.1];
        palette
    }

    /// One unit cube: exactly the engine mesher's output for a single cell, so a
    /// placement marks exactly the cell its translation names.
    fn unit_cube() -> Mesh {
        let mut voxel = World::new(0);
        voxel.set([0, 0, 0], 1);
        voxel.mesh()
    }

    fn proxy(origin: [i32; 3], dimensions: [u32; 3], cells: &[([i32; 3], u8)]) -> MeshProxy {
        let mut meshes = Vec::new();
        let mut materials = Vec::new();
        let mut instances = Vec::new();
        for &(cell, material) in cells {
            let prototype = match materials.iter().position(|&known| known == material) {
                Some(index) => index,
                None => {
                    meshes.push(unit_cube());
                    materials.push(material);
                    meshes.len() - 1
                }
            };
            instances.push(StaticInstance {
                prototype,
                translation: cell.map(|value| value as f32),
                yaw_quarters: 0,
            });
        }
        MeshProxy::build(
            &MeshGeometry {
                meshes: &meshes,
                instances: &instances,
                materials: &materials,
            },
            origin,
            dimensions,
        )
        .expect("test proxy build")
    }

    fn cells_in(origin: [i32; 3], dimensions: [u32; 3]) -> impl Iterator<Item = [i32; 3]> {
        (0..dimensions[2]).flat_map(move |z| {
            (0..dimensions[1]).flat_map(move |y| {
                (0..dimensions[0]).map(move |x| {
                    [
                        origin[0] + x as i32,
                        origin[1] + y as i32,
                        origin[2] + z as i32,
                    ]
                })
            })
        })
    }

    /// Floor plus six scattered single cells, all in the proxy: the wetland's
    /// own shape is mesh-only, so the proxy owns the whole scene here.
    fn sparse_scene() -> Vec<([i32; 3], u8)> {
        let mut cells: Vec<([i32; 3], u8)> = cells_in(BOX_ORIGIN, BOX_DIMENSIONS)
            .filter(|cell| cell[1] == 0)
            .map(|cell| (cell, FLOOR_MATERIAL))
            .collect();
        for cell in [
            [-6, 1, -3],
            [-2, 1, 4],
            [3, 1, -6],
            [6, 1, 7],
            [-4, 1, 6],
            [7, 1, 1],
        ] {
            cells.push((cell, ROCK_MATERIAL));
        }
        cells
    }

    /// Floor plus a checkerboard of two-cell pillars every other cell.
    fn dense_scene() -> Vec<([i32; 3], u8)> {
        let mut cells: Vec<([i32; 3], u8)> = Vec::new();
        for cell in cells_in(BOX_ORIGIN, BOX_DIMENSIONS) {
            let even = |axis: usize| (cell[axis] - BOX_ORIGIN[axis]) % 2 == 0;
            if cell[1] == 0 || (cell[1] > 0 && cell[1] < 3 && even(0) && even(2)) {
                cells.push((
                    cell,
                    if cell[1] == 0 {
                        FLOOR_MATERIAL
                    } else {
                        ROCK_MATERIAL
                    },
                ));
            }
        }
        cells
    }

    /// Sixty adjacent cells of a rectangle ring inside the box, so a frame's
    /// body move is always a one-cell move.
    fn ring_path() -> Vec<[i32; 3]> {
        let mut path = Vec::new();
        for x in -8..=8 {
            path.push([x, 3, -8]);
        }
        for z in -7..=8 {
            path.push([8, 3, z]);
        }
        for x in (-8..8).rev() {
            path.push([x, 3, 8]);
        }
        for z in (-7..8).rev() {
            path.push([-8, 3, z]);
        }
        path
    }

    fn volume(origin: [i32; 3], dimensions: [u32; 3], distance: f32) -> IndirectVolume {
        IndirectVolume::new(origin, dimensions, SAMPLES, distance, palette()).expect("test volume")
    }

    fn retained_volume(origin: [i32; 3], dimensions: [u32; 3], distance: f32) -> IndirectVolume {
        let mut volume = volume(origin, dimensions, distance);
        volume
            .enable_proxy_retention(TRACKING_CAP)
            .expect("test tracking");
        volume
    }

    /// Drive one volume to `complete()` with the wetland budget, counting real
    /// slices, rays and work units.
    fn complete(volume: &mut IndirectVolume, world: &World, sun: Sun) -> (usize, usize, usize) {
        let (mut frames, mut rays, mut work) = (0, 0, 0);
        while !volume.complete() {
            let stats = volume
                .update(world, 0, sun, UPDATE_BUDGET)
                .expect("test update");
            frames += 1;
            rays += stats.rays;
            work += stats.work;
            assert!(frames < 100_000, "volume never completed");
        }
        (frames, rays, work)
    }

    fn all_slots(
        origin: [i32; 3],
        dimensions: [u32; 3],
    ) -> impl Iterator<Item = ([i32; 3], usize)> {
        cells_in(origin, dimensions).flat_map(|cell| (0..6).map(move |face| (cell, face)))
    }

    /// Bit-exact equality of every published face. This is the oracle: a
    /// retained face and a face recomputed from its first sample are both the
    /// same deterministic quadrature over the same geometry.
    fn assert_matches_fresh(
        measured: &IndirectVolume,
        fresh: &IndirectVolume,
        origin: [i32; 3],
        dimensions: [u32; 3],
        label: &str,
    ) {
        let mut compared = 0usize;
        for (cell, face) in all_slots(origin, dimensions) {
            let a = measured.sample(cell, face);
            let b = fresh.sample(cell, face);
            assert!(
                a[0].to_bits() == b[0].to_bits()
                    && a[1].to_bits() == b[1].to_bits()
                    && a[2].to_bits() == b[2].to_bits(),
                "{label}: {cell:?} face {face} differs: retained {a:?} vs fresh {b:?}"
            );
            compared += 1;
        }
        assert_eq!(compared, dimensions.iter().product::<u32>() as usize * 6);
    }

    /// The retention contract, checked slot by slot through the completed
    /// marker: an invalidated slot is zeroed and counted, a retained slot keeps
    /// its exact value and is counted, and the reported retained/dirty counts
    /// are the real ones.
    fn assert_retention_invariants(
        before: &[[f32; 4]],
        volume: &IndirectVolume,
        edit: &ProxyEdit,
        label: &str,
    ) {
        let after = &volume.values;
        assert_eq!(before.len(), after.len());
        let (mut kept, mut invalidated) = (0, 0);
        for (before, after) in before.iter().zip(after) {
            if after[3] == 0.0 {
                assert_eq!(
                    after, &[0.; 4],
                    "{label}: a slot without a current value must be zeroed, not stale"
                );
                if before[3] != 0.0 {
                    invalidated += 1;
                }
            } else {
                assert_eq!(
                    before, after,
                    "{label}: a retained slot must keep its exact value"
                );
                kept += 1;
            }
        }
        let dirty = (0..after.len())
            .filter(|&slot| !bit_is_set(&volume.done, slot))
            .count();
        assert_eq!(invalidated, edit.invalidated_faces, "{label}: invalidated");
        assert_eq!(kept, edit.retained_faces, "{label}: retained");
        assert_eq!(dirty, edit.dirty_faces, "{label}: dirty");
    }

    fn snapshot(volume: &IndirectVolume) -> Vec<[f32; 4]> {
        volume.values.clone()
    }

    /// A retained volume and an untracked volume built from the same proxy must
    /// publish the same values.
    fn fresh_reference(
        origin: [i32; 3],
        dimensions: [u32; 3],
        distance: f32,
        world: &World,
        sun: Sun,
        mesh: MeshProxy,
    ) -> IndirectVolume {
        let mut fresh = volume(origin, dimensions, distance);
        fresh.set_mesh_proxy(Some(mesh));
        complete(&mut fresh, world, sun);
        fresh
    }

    /// The walker must visit exactly the cells the shipped DDA reads: the path
    /// from the origin cell up to and including the first solid cell, or the
    /// whole path within the limit when nothing is solid. This pins the mirror
    /// against `MeshProxy::raycast` (and, through the identical cells, against
    /// `World::raycast`).
    #[test]
    fn walker_reads_the_cells_the_dda_reads() {
        let origin = [0, 0, 0];
        let dimensions = [12u32, 5, 12];
        let mut cells = Vec::new();
        for cell in cells_in(origin, dimensions) {
            if cell[1] == 0 {
                cells.push((cell, FLOOR_MATERIAL));
            }
            let even = |axis: usize| (cell[axis] - origin[axis]) % 2 == 0;
            if cell[1] == 1 && even(0) && even(2) {
                cells.push((cell, ROCK_MATERIAL));
            }
        }
        let mesh = proxy(origin, dimensions, &cells);
        // The same cells as an authoritative world: one DDA implementation, two
        // entry points, so this also checks the proxy's box clipping is the only
        // difference.
        let mut world = World::new(0);
        for &(cell, material) in &cells {
            world.set(cell, material);
        }
        let mut visited = Vec::new();
        let mut rays = 0usize;
        let mut hits = 0usize;
        for cell in cells_in(origin, dimensions) {
            for normal_cell in FACE_NORMALS {
                if scene_material(&world, Some(&mesh), cell) == 0
                    || scene_material(
                        &world,
                        Some(&mesh),
                        [
                            cell[0] + normal_cell[0],
                            cell[1] + normal_cell[1],
                            cell[2] + normal_cell[2],
                        ],
                    ) != 0
                {
                    continue;
                }
                let normal = Vec3::from_array(normal_cell.map(|n| n as f32));
                let face_origin = Vec3::from_array(cell.map(|value| value as f32))
                    + Vec3::splat(0.5)
                    + normal * (0.5 + 0.001);
                for sample in 0..SAMPLES {
                    let ray = crate::indirect::hemisphere(normal, sample, SAMPLES);
                    for (direction, max_distance) in [
                        (ray, 24.0f32),
                        (Vec3::from_array(SUN.direction_to_sun).normalize(), 24.0f32),
                    ] {
                        rays += 1;
                        visited.clear();
                        let limit = world
                            .raycast(face_origin.to_array(), direction.to_array(), max_distance)
                            .map_or(max_distance, |hit| hit.distance);
                        walk_segment(
                            |cell| mesh.material_at(cell),
                            face_origin.to_array(),
                            direction.to_array(),
                            limit,
                            |cell| visited.push(cell),
                        );
                        let expected =
                            mesh.raycast(face_origin.to_array(), direction.to_array(), limit);
                        let start = [
                            face_origin.x.floor() as i32,
                            face_origin.y.floor() as i32,
                            face_origin.z.floor() as i32,
                        ];
                        assert_eq!(
                            visited.first().copied(),
                            Some(start),
                            "the walk starts in the origin's cell"
                        );
                        for pair in visited.windows(2) {
                            for axis in 0..3 {
                                assert!(
                                    (pair[0][axis] - pair[1][axis]).abs() <= 1,
                                    "visited cells must be a unit-step path: {pair:?}"
                                );
                            }
                            assert_ne!(pair[0], pair[1], "a step must change a cell");
                        }
                        let Some(exit) =
                            mesh.segment_exit(face_origin.to_array(), direction.to_array(), limit)
                        else {
                            // A segment that cannot enter the coverage box reads
                            // no proxy cell at all, so it depends on none.
                            assert_eq!(expected, None);
                            assert!(
                                visited.iter().all(|&cell| mesh.material_at(cell) == 0),
                                "a segment outside the box must not read a cell"
                            );
                            continue;
                        };
                        assert_eq!(
                            world.raycast(face_origin.to_array(), direction.to_array(), exit),
                            expected,
                            "the proxy DDA must read the same cells as the world DDA"
                        );
                        match expected {
                            Some(hit) => {
                                hits += 1;
                                assert_eq!(
                                    visited.last().copied(),
                                    Some(hit.cell),
                                    "the walk stops at the first solid cell"
                                );
                            }
                            None => {
                                for &cell in &visited {
                                    assert_eq!(
                                        mesh.material_at(cell),
                                        0,
                                        "a miss must not pass through solid cells"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(rays > 10_000, "fixture must exercise many segments");
        assert!(hits > 100, "fixture must exercise solid hits: {hits}");
    }

    /// The two-segment counterexample from the moving-lighting probe: the moved
    /// body is more than `R` cells away from the receiver and no gather ray can
    /// reach it, so only the hit-to-sun segment can see the change. Exact
    /// dependency tracking must invalidate the receiver because its recorded
    /// sun segment passed through the changed cell, and the completed volume
    /// must equal a fresh one.
    #[test]
    fn far_occluder_second_segment_is_invalidated_exactly() {
        const CX_ORIGIN: [i32; 3] = [0, 0, 0];
        const CX_DIMENSIONS: [u32; 3] = [10, 5, 20];
        const CX_RADIUS_M: f32 = 4.0;
        const RECEIVER: [i32; 3] = [4, 1, 8];
        const RECEIVER_FACE: usize = 4;
        const BODY_BEFORE: [i32; 3] = [3, 1, 14];
        const BODY_AFTER: [i32; 3] = [3, 1, 13];
        let mut world = World::new(47);
        for cell in cells_in(CX_ORIGIN, CX_DIMENSIONS) {
            if cell[1] == 0 {
                world.set(cell, FLOOR_MATERIAL);
            }
        }
        world.set(RECEIVER, ROCK_MATERIAL);
        let body = |cell: [i32; 3]| proxy(CX_ORIGIN, CX_DIMENSIONS, &[(cell, BODY_MATERIAL)]);
        let mut measured = retained_volume(CX_ORIGIN, CX_DIMENSIONS, CX_RADIUS_M);
        measured.set_mesh_proxy(Some(body(BODY_BEFORE)));
        complete(&mut measured, &world, SUN);
        let before = snapshot(&measured);
        let lit = measured.sample(RECEIVER, RECEIVER_FACE);
        assert!(lit[0] > 0.0, "fixture: the receiver must gather the floor");

        let edit = measured.replace_mesh_proxy(Some(body(BODY_AFTER)));
        assert!(edit.tracked, "the edit is inside the index space");
        assert_eq!(edit.changed_cells, 2, "a one-cell move changes two cells");
        assert!(
            edit.invalidated_faces > 0,
            "the second segment must invalidate at least the receiver"
        );
        assert_retention_invariants(&before, &measured, &edit, "far occluder");
        assert!(
            !measured.complete(),
            "an edit must not publish before every face is finished"
        );
        assert!(!measured.valid_for(&world, 0, SUN));
        assert!(!measured.source_valid(&world, 0));
        let mid = measured.sample(RECEIVER, RECEIVER_FACE);
        assert_eq!(
            mid, [0.; 3],
            "the invalidated receiver must be zero, never stale: {mid:?}"
        );
        complete(&mut measured, &world, SUN);
        let occluded = measured.sample(RECEIVER, RECEIVER_FACE);
        assert!(
            occluded[0] > 0.0 && occluded[0] < lit[0],
            "the closer body must block part of the receiver's sun: {lit:?} -> {occluded:?}"
        );
        let fresh = fresh_reference(
            CX_ORIGIN,
            CX_DIMENSIONS,
            CX_RADIUS_M,
            &world,
            SUN,
            body(BODY_AFTER),
        );
        assert_matches_fresh(&measured, &fresh, CX_ORIGIN, CX_DIMENSIONS, "far occluder");

        // Control: reverse the sun's horizontal component. No gather ray can
        // reach either body cell, so the move may not change the receiver; this
        // isolates the second segment as the source of the change above.
        const REVERSED: Sun = Sun {
            direction_to_sun: [0.0, 1.0, -5.0],
            intensity: 1.0,
        };
        let mut control = retained_volume(CX_ORIGIN, CX_DIMENSIONS, CX_RADIUS_M);
        control.set_mesh_proxy(Some(body(BODY_BEFORE)));
        complete(&mut control, &world, REVERSED);
        let control_lit = control.sample(RECEIVER, RECEIVER_FACE);
        control.replace_mesh_proxy(Some(body(BODY_AFTER)));
        complete(&mut control, &world, REVERSED);
        assert_eq!(
            control.sample(RECEIVER, RECEIVER_FACE)[0].to_bits(),
            control_lit[0].to_bits(),
            "with the occluder left behind, the second segment must not change"
        );
    }

    /// An edit that encloses a sampled face changes its exposure, not just its
    /// rays: the face must be zeroed immediately (never left stale) and the
    /// completed volume must match the fresh reference without that face.
    #[test]
    fn enclosing_edit_zeroes_the_newly_hidden_face() {
        let world = World::new(0);
        let mut cells = sparse_scene();
        cells.push(([0, 3, 0], BODY_MATERIAL));
        let mut measured = retained_volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
        measured.set_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)));
        complete(&mut measured, &world, SUN);
        // The body's -Z face gathers the scene in front of it.
        let face = 5;
        let lit = measured.sample([0, 3, 0], face);
        assert!(lit[0] > 0.0, "fixture: the face must start lit: {lit:?}");
        assert!(measured.complete() && measured.valid_for(&world, 0, SUN));

        // A wall one cell closer in -Z encloses that face.
        let mut enclosed = cells.clone();
        enclosed.push(([0, 3, -1], ROCK_MATERIAL));
        let before = snapshot(&measured);
        let edit = measured.replace_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &enclosed)));
        assert!(edit.tracked);
        assert_eq!(edit.changed_cells, 1);
        assert!(
            edit.invalidated_faces > 0,
            "the exposure change must invalidate"
        );
        assert_retention_invariants(&before, &measured, &edit, "enclosed");
        assert!(!measured.complete());
        assert!(!measured.valid_for(&world, 0, SUN));
        assert!(!measured.source_valid(&world, 0));
        assert_eq!(
            measured.sample([0, 3, 0], face),
            [0.; 3],
            "a face that just lost its air neighbor must be zero, not stale"
        );
        complete(&mut measured, &world, SUN);
        assert_eq!(
            measured.sample([0, 3, 0], face),
            [0.; 3],
            "the enclosed face is not sampled at all"
        );
        assert!(measured.valid_for(&world, 0, SUN) && measured.source_valid(&world, 0));
        let fresh = fresh_reference(
            BOX_ORIGIN,
            BOX_DIMENSIONS,
            GATHER_DISTANCE_M,
            &world,
            SUN,
            proxy(BOX_ORIGIN, BOX_DIMENSIONS, &enclosed),
        );
        assert_matches_fresh(&measured, &fresh, BOX_ORIGIN, BOX_DIMENSIONS, "enclosed");
    }

    /// Move, material edit, removal and insertion: after each single-cell edit
    /// the retained volume completes to a bit-exact copy of a fresh volume, and
    /// most completed faces are retained rather than recomputed.
    #[test]
    fn single_cell_edits_match_a_fresh_reference() {
        for (label, scene) in [("sparse", sparse_scene()), ("dense", dense_scene())] {
            let world = World::new(0);
            let body_cell = [0, 3, 0];
            let mut cells = scene.clone();
            cells.push((body_cell, BODY_MATERIAL));
            let mut measured = retained_volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
            measured.set_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)));
            complete(&mut measured, &world, SUN);
            let untouched = snapshot(&measured);
            let status = measured.retention_status().expect("tracking status");
            println!(
                "[retention] {label} tracking: tracked={} untracked={} outside_cells={} \
checked={} bytes={}",
                status.tracked_faces,
                status.untracked_faces,
                status.outside_cells,
                status.bits_per_face,
                status.resident_bytes,
            );
            assert!(
                status.tracked_faces > 0,
                "the fixture must sample faces through the tracker: {status:?}"
            );
            assert!(
                status.outside_cells > 0,
                "coverage-box boundary faces read cells outside the index space: {status:?}"
            );
            assert!(status.resident_bytes <= TRACKING_CAP);

            // Move the body one cell.
            let mut moved_cells = scene.clone();
            moved_cells.push(([1, 3, 0], BODY_MATERIAL));
            let edit =
                measured.replace_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &moved_cells)));
            println!("[retention] {label} move: {edit:?}");
            assert_eq!(edit.changed_cells, 2, "{label}: move diff");
            assert!(
                edit.retained_faces > edit.invalidated_faces,
                "{label}: a single-cell move must retain most faces: {edit:?}"
            );
            assert_retention_invariants(&untouched, &measured, &edit, label);
            complete(&mut measured, &world, SUN);
            let fresh = fresh_reference(
                BOX_ORIGIN,
                BOX_DIMENSIONS,
                GATHER_DISTANCE_M,
                &world,
                SUN,
                proxy(BOX_ORIGIN, BOX_DIMENSIONS, &moved_cells),
            );
            assert_matches_fresh(&measured, &fresh, BOX_ORIGIN, BOX_DIMENSIONS, label);

            // Material edit: the body keeps its cell, its albedo changes.
            let mut edited_cells = scene.clone();
            edited_cells.push(([1, 3, 0], ALT_MATERIAL));
            let edit =
                measured.replace_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &edited_cells)));
            println!("[retention] {label} material: {edit:?}");
            assert_eq!(edit.changed_cells, 1, "{label}: material diff");
            assert!(
                edit.invalidated_faces > 0,
                "{label}: a material edit must invalidate the faces that see it"
            );
            assert!(edit.retained_faces > edit.invalidated_faces);
            complete(&mut measured, &world, SUN);
            let fresh = fresh_reference(
                BOX_ORIGIN,
                BOX_DIMENSIONS,
                GATHER_DISTANCE_M,
                &world,
                SUN,
                proxy(BOX_ORIGIN, BOX_DIMENSIONS, &edited_cells),
            );
            assert_matches_fresh(&measured, &fresh, BOX_ORIGIN, BOX_DIMENSIONS, label);

            // Removal: the body disappears, its old hit cells must invalidate.
            let mut removed_cells = scene.clone();
            removed_cells.push(([2, 3, 3], BODY_MATERIAL));
            let mut with_second = retained_volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
            with_second.set_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &removed_cells)));
            complete(&mut with_second, &world, SUN);
            let before_removal = snapshot(&with_second);
            let mut without = removed_cells.clone();
            without.pop();
            let edit =
                with_second.replace_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &without)));
            println!("[retention] {label} removal: {edit:?}");
            assert_eq!(edit.changed_cells, 1, "{label}: removal diff");
            assert!(
                edit.invalidated_faces > 0,
                "{label}: removing a hit must invalidate its receivers"
            );
            assert_retention_invariants(&before_removal, &with_second, &edit, label);
            complete(&mut with_second, &world, SUN);
            let fresh = fresh_reference(
                BOX_ORIGIN,
                BOX_DIMENSIONS,
                GATHER_DISTANCE_M,
                &world,
                SUN,
                proxy(BOX_ORIGIN, BOX_DIMENSIONS, &without),
            );
            assert_matches_fresh(&with_second, &fresh, BOX_ORIGIN, BOX_DIMENSIONS, label);

            // Insertion into a previously empty ray path.
            let mut inserted_cells = without.clone();
            inserted_cells.push(([2, 4, 3], BODY_MATERIAL));
            let edit = with_second.replace_mesh_proxy(Some(proxy(
                BOX_ORIGIN,
                BOX_DIMENSIONS,
                &inserted_cells,
            )));
            println!("[retention] {label} insertion: {edit:?}");
            assert_eq!(edit.changed_cells, 1, "{label}: insertion diff");
            assert!(
                edit.invalidated_faces > 0,
                "{label}: inserting an occluder on an empty path must invalidate"
            );
            complete(&mut with_second, &world, SUN);
            let fresh = fresh_reference(
                BOX_ORIGIN,
                BOX_DIMENSIONS,
                GATHER_DISTANCE_M,
                &world,
                SUN,
                proxy(BOX_ORIGIN, BOX_DIMENSIONS, &inserted_cells),
            );
            assert_matches_fresh(&with_second, &fresh, BOX_ORIGIN, BOX_DIMENSIONS, label);
        }
    }

    /// A second edit while the first is still recomputing must accumulate
    /// invalidation. Faces invalidated by the first edit but not yet recomputed
    /// are resampled against the second representation, and the completed volume
    /// still equals a fresh one for the final proxy.
    #[test]
    fn overlapping_edits_accumulate_invalidation() {
        let world = World::new(0);
        let mut measured = retained_volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
        let mut cells = sparse_scene();
        cells.push(([0, 3, 0], BODY_MATERIAL));
        measured.set_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)));
        complete(&mut measured, &world, SUN);

        let mut first = sparse_scene();
        first.push(([1, 3, 0], BODY_MATERIAL));
        measured.replace_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &first)));
        // One tiny slice, deliberately far from completing the first edit.
        let partial = measured
            .update(&world, 0, SUN, UpdateBudget { rays: 32, work: 4 })
            .expect("partial slice");
        assert!(!partial.complete, "the first edit must still be pending");
        assert!(measured.pending_work() > 0);

        let mut second = sparse_scene();
        second.push(([2, 3, 0], BODY_MATERIAL));
        let edit = measured.replace_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &second)));
        assert!(edit.tracked);
        assert_eq!(edit.changed_cells, 2, "the second move changes two cells");
        assert!(
            !measured.complete(),
            "the volume must not complete by discarding the first edit's work"
        );
        assert!(!measured.valid_for(&world, 0, SUN));
        complete(&mut measured, &world, SUN);
        let fresh = fresh_reference(
            BOX_ORIGIN,
            BOX_DIMENSIONS,
            GATHER_DISTANCE_M,
            &world,
            SUN,
            proxy(BOX_ORIGIN, BOX_DIMENSIONS, &second),
        );
        assert_matches_fresh(
            &measured,
            &fresh,
            BOX_ORIGIN,
            BOX_DIMENSIONS,
            "overlapping edits",
        );

        // A superseded key may not publish: the volume was keyed to the first
        // edit's proxy while it was pending, and completing the second key is
        // the only state that is publishable.
        assert!(measured.valid_for(&world, 0, SUN));
        assert!(measured.source_valid(&world, 0));
    }

    /// World revision/epoch and sun are not part of the retention record: any of
    /// them changing must clear every retained value.
    #[test]
    fn world_sun_and_epoch_changes_clear_retained_values() {
        let mut world = World::new(0);
        let mut measured = retained_volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
        let mut cells = sparse_scene();
        cells.push(([0, 3, 0], BODY_MATERIAL));
        measured.set_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)));
        complete(&mut measured, &world, SUN);
        let completed = snapshot(&measured);
        let retained_values = completed.iter().filter(|value| value[3] != 0.0).count();
        assert!(retained_values > 0);

        // Sun change: values computed for another light may not survive.
        measured
            .update(&world, 0, SUN_OTHER, UpdateBudget { rays: 0, work: 0 })
            .expect("sun key change");
        assert!(
            snapshot(&measured).iter().all(|value| *value == [0.; 4]),
            "a sun change must clear every retained value"
        );
        assert!(!measured.complete());
        complete(&mut measured, &world, SUN_OTHER);
        let fresh = fresh_reference(
            BOX_ORIGIN,
            BOX_DIMENSIONS,
            GATHER_DISTANCE_M,
            &world,
            SUN_OTHER,
            proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells),
        );
        assert_matches_fresh(&measured, &fresh, BOX_ORIGIN, BOX_DIMENSIONS, "sun change");

        // World revision change: the authoritative world is not re-recorded per
        // face, so a world edit must clear everything.
        complete(&mut measured, &world, SUN_OTHER);
        world.set([5, 1, 5], ROCK_MATERIAL);
        measured
            .update(&world, 0, SUN_OTHER, UpdateBudget { rays: 0, work: 0 })
            .expect("world key change");
        assert!(
            snapshot(&measured).iter().all(|value| *value == [0.; 4]),
            "a world change must clear every retained value"
        );

        // Replacement epoch change with an identical revision.
        complete(&mut measured, &world, SUN_OTHER);
        measured
            .update(&world, 1, SUN_OTHER, UpdateBudget { rays: 0, work: 0 })
            .expect("epoch key change");
        assert!(
            snapshot(&measured).iter().all(|value| *value == [0.; 4]),
            "an epoch change must clear every retained value"
        );
    }

    /// A changed cell outside the bounded index space is a coverage change: the
    /// diff cannot be represented, so the volume uses its clear-all semantics
    /// and reports that it did.
    #[test]
    fn coverage_change_outside_the_index_space_clears() {
        let world = World::new(0);
        // A small volume box with a proxy whose box is larger: the body can move
        // through cells the volume cannot index.
        let origin = [0, 0, 0];
        let dimensions = [6u32, 3, 6];
        let proxy_origin = [0, 0, 0];
        let proxy_dimensions = [10u32, 3, 10];
        let mut measured = retained_volume(origin, dimensions, GATHER_DISTANCE_M);
        let mut cells: Vec<([i32; 3], u8)> = cells_in(origin, dimensions)
            .filter(|cell| cell[1] == 0)
            .map(|cell| (cell, FLOOR_MATERIAL))
            .collect();
        cells.push(([2, 1, 2], BODY_MATERIAL));
        measured.set_mesh_proxy(Some(proxy(proxy_origin, proxy_dimensions, &cells)));
        complete(&mut measured, &world, SUN);
        let before = snapshot(&measured);
        assert!(before.iter().any(|value| value[3] != 0.0));

        let mut moved = cells.clone();
        moved.pop();
        moved.push(([8, 1, 8], BODY_MATERIAL));
        let edit = measured.replace_mesh_proxy(Some(proxy(proxy_origin, proxy_dimensions, &moved)));
        assert!(!edit.tracked, "an out-of-index change is a coverage change");
        assert_eq!(edit.changed_cells, 2);
        assert_eq!(
            edit.invalidated_faces,
            before.iter().filter(|v| v[3] != 0.0).count()
        );
        assert_eq!(edit.retained_faces, 0);
        assert_eq!(edit.dirty_faces, measured.values.len());
        assert!(
            snapshot(&measured).iter().all(|value| *value == [0.; 4]),
            "a coverage change must clear every value"
        );
        assert!(!measured.complete());

        // The in-box part of the change still converges to the fresh reference.
        complete(&mut measured, &world, SUN);
        let fresh = fresh_reference(
            origin,
            dimensions,
            GATHER_DISTANCE_M,
            &world,
            SUN,
            proxy(proxy_origin, proxy_dimensions, &moved),
        );
        assert_matches_fresh(&measured, &fresh, origin, dimensions, "coverage change");
    }

    /// A proxy box larger than the volume box is still exactly trackable as
    /// long as its occupied cells are inside the index space: segments may
    /// traverse empty cells outside the volume box, but those cannot change
    /// without a coverage change, so retained values stay correct.
    #[test]
    fn out_of_box_traversal_cells_do_not_block_retention() {
        let world = World::new(0);
        let origin = [0, 0, 0];
        let dimensions = [6u32, 3, 6];
        let proxy_origin = [0, 0, 0];
        let proxy_dimensions = [10u32, 3, 10];
        let mut cells: Vec<([i32; 3], u8)> = cells_in(origin, dimensions)
            .filter(|cell| cell[1] == 0)
            .map(|cell| (cell, FLOOR_MATERIAL))
            .collect();
        cells.push(([2, 2, 2], BODY_MATERIAL));
        let mut measured = retained_volume(origin, dimensions, GATHER_DISTANCE_M);
        measured.set_mesh_proxy(Some(proxy(proxy_origin, proxy_dimensions, &cells)));
        complete(&mut measured, &world, SUN);
        let before = snapshot(&measured);
        assert!(
            measured.retention_status().expect("status").outside_cells > 0,
            "the larger proxy box must make segments traverse outside the volume box"
        );

        let mut moved = cells.clone();
        moved.pop();
        moved.push(([3, 2, 2], BODY_MATERIAL));
        let edit = measured.replace_mesh_proxy(Some(proxy(proxy_origin, proxy_dimensions, &moved)));
        assert!(edit.tracked, "in-index changes are tracked");
        assert_eq!(edit.changed_cells, 2);
        assert!(edit.retained_faces > 0);
        assert_retention_invariants(&before, &measured, &edit, "out-of-box traversal");
        complete(&mut measured, &world, SUN);
        let fresh = fresh_reference(
            origin,
            dimensions,
            GATHER_DISTANCE_M,
            &world,
            SUN,
            proxy(proxy_origin, proxy_dimensions, &moved),
        );
        assert_matches_fresh(
            &measured,
            &fresh,
            origin,
            dimensions,
            "out-of-box traversal",
        );
    }

    /// A tracking cap that cannot hold every sampled face must degrade to
    /// always-recomputed faces, never to a retained stale value, and the volume
    /// still completes to the fresh reference.
    #[test]
    fn tracking_cap_degrades_to_untracked_never_stale() {
        let world = World::new(0);
        let mut cells = sparse_scene();
        cells.push(([0, 3, 0], BODY_MATERIAL));
        let mut volume = volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
        let probe = volume
            .enable_proxy_retention(TRACKING_CAP)
            .expect("tracking probe");
        let words = probe.bits_per_face.div_ceil(64);
        let per_face = words * 8;
        let fixed = probe.face_slots * 4 + words * 8;
        // Room for the fixed arrays and eight face bitsets only.
        let cap = fixed + 8 * per_face;
        let status = volume
            .enable_proxy_retention(cap)
            .expect("small tracking cap");
        assert_eq!(status.cap_bytes, cap);
        volume.set_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)));
        complete(&mut volume, &world, SUN);
        let before = snapshot(&volume);
        let completed = before.iter().filter(|value| value[3] != 0.0).count();
        let live = volume.retention_status().expect("tracking status");
        assert!(
            live.untracked_faces > 0,
            "the fixture must exhaust the cap: {live:?}"
        );
        assert!(live.tracked_faces < completed, "most faces stay untracked");
        assert!(
            live.resident_bytes <= cap + 1024,
            "resident {} over cap {cap}",
            live.resident_bytes
        );
        assert!(live.bits_per_face == probe.bits_per_face);

        let mut moved = sparse_scene();
        moved.push(([1, 3, 0], BODY_MATERIAL));
        let edit = volume.replace_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &moved)));
        assert!(edit.tracked);
        assert!(edit.changed_cells > 0);
        assert!(
            edit.retained_faces <= live.tracked_faces,
            "only tracked faces may be retained: {edit:?} vs {live:?}"
        );
        assert!(
            edit.invalidated_faces >= completed - live.tracked_faces,
            "every completed untracked face must be invalidated: {edit:?}"
        );
        assert_retention_invariants(&before, &volume, &edit, "small cap");
        assert!(!volume.complete());
        complete(&mut volume, &world, SUN);
        let fresh = fresh_reference(
            BOX_ORIGIN,
            BOX_DIMENSIONS,
            GATHER_DISTANCE_M,
            &world,
            SUN,
            proxy(BOX_ORIGIN, BOX_DIMENSIONS, &moved),
        );
        assert_matches_fresh(&volume, &fresh, BOX_ORIGIN, BOX_DIMENSIONS, "small cap");

        // The caller's cap is clamped to the engine maximum, never trusted.
        let clamped = volume
            .enable_proxy_retention(usize::MAX)
            .expect("clamped tracking");
        assert_eq!(clamped.cap_bytes, MAX_DEPENDENCY_BYTES);
    }

    /// Replacing the proxy with an identical footprint changes nothing: the
    /// complete volume stays complete and no work is scheduled.
    #[test]
    fn identical_proxy_edit_keeps_the_complete_volume() {
        let world = World::new(0);
        let mut cells = sparse_scene();
        cells.push(([0, 3, 0], BODY_MATERIAL));
        let mut volume = retained_volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
        volume.set_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)));
        complete(&mut volume, &world, SUN);
        let before = snapshot(&volume);
        let edit = volume.replace_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)));
        assert!(edit.tracked);
        assert_eq!(edit.changed_cells, 0);
        assert_eq!(edit.invalidated_faces, 0);
        assert_eq!(
            edit.retained_faces,
            before.iter().filter(|v| v[3] != 0.0).count()
        );
        assert!(
            volume.complete(),
            "an identical proxy must not schedule work"
        );
        assert_eq!(volume.pending_work(), 0);
        assert_eq!(snapshot(&volume), before);
    }

    /// The old clear-all API keeps its behavior even with tracking enabled.
    #[test]
    fn old_set_mesh_proxy_still_clears() {
        let world = World::new(0);
        let mut cells = sparse_scene();
        cells.push(([0, 3, 0], BODY_MATERIAL));
        let mut volume = retained_volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
        volume.set_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)));
        complete(&mut volume, &world, SUN);
        assert!(volume.complete());
        volume.set_mesh_proxy(Some(proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)));
        assert!(!volume.complete(), "set_mesh_proxy clears unconditionally");
        assert!(snapshot(&volume).iter().all(|value| *value == [0.; 4]));
        complete(&mut volume, &world, SUN);
        let fresh = fresh_reference(
            BOX_ORIGIN,
            BOX_DIMENSIONS,
            GATHER_DISTANCE_M,
            &world,
            SUN,
            proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells),
        );
        assert_matches_fresh(
            &volume,
            &fresh,
            BOX_ORIGIN,
            BOX_DIMENSIONS,
            "set_mesh_proxy",
        );
    }

    /// The coverage box clips the proxy trace: geometry outside it cannot affect
    /// a segment that stays inside, and a proxy whose box excludes an occluder
    /// cannot hit it even when the ray, distance and proxy cells are identical.
    /// This is why the dependency index space can be the volume's own box.
    #[test]
    fn outside_coverage_geometry_is_clipped_from_the_trace() {
        let world = World::new(0);
        let small = [0i32, 0, 0];
        let small_dimensions = [12u32, 4, 12];
        let large = [0i32, 0, 0];
        let large_dimensions = [24u32, 4, 12];
        let wall = [14i32, 1, 5];
        let inside_wall = [6i32, 1, 5];
        let small_cells = [(inside_wall, ROCK_MATERIAL)];
        let large_cells = [(inside_wall, ROCK_MATERIAL), (wall, ROCK_MATERIAL)];
        let clipped = proxy(small, small_dimensions, &small_cells);
        let containing = proxy(large, large_dimensions, &large_cells);
        let origin = [2.5f32, 1.5, 5.5];
        let direction = [1.0f32, 0.0, 0.0];
        let inside = clipped.raycast(origin, direction, 100.0);
        assert_eq!(
            inside.map(|hit| hit.cell),
            Some(inside_wall),
            "the in-box wall is hit identically"
        );
        let far = containing.raycast(origin, direction, 100.0);
        assert_eq!(
            far.map(|hit| hit.cell),
            Some(inside_wall),
            "the containing box still stops at the first wall"
        );
        // Remove the in-box wall: the small box cannot see the wall beyond its
        // coverage, the larger box can.
        let no_near = proxy(small, small_dimensions, &[]);
        let far_only = proxy(large, large_dimensions, &[(wall, ROCK_MATERIAL)]);
        assert_eq!(
            no_near.raycast(origin, direction, 100.0),
            None,
            "geometry outside the coverage box must not be hit"
        );
        assert_eq!(
            far_only
                .raycast(origin, direction, 100.0)
                .map(|hit| hit.cell),
            Some(wall),
            "the larger box reaches the same cells"
        );
        // Identical cells, identical traces: the box only bounds reach.
        let same_a = proxy(small, small_dimensions, &small_cells);
        let same_b = proxy(small, small_dimensions, &small_cells);
        for t in 0..20 {
            let direction = [1.0f32, 0.1 * t as f32, 0.05 * t as f32];
            assert_eq!(
                same_a.raycast(origin, direction, 100.0),
                same_b.raycast(origin, direction, 100.0)
            );
        }
        let _ = world;
    }

    /// Sixty one-cell moves: report the actual retention per frame, the real
    /// ray/work cost of exact dependency invalidation against whole-volume
    /// invalidation on sparse and dense fixtures, and prove the moving volume
    /// converges to the fresh reference. No wall-clock and no universal claim.
    #[test]
    fn continuous_motion_retains_most_faces_and_converges() {
        let world = World::new(0);
        let path = ring_path();
        assert!(path.len() >= 60);
        for (label, scene) in [("sparse", sparse_scene()), ("dense", dense_scene())] {
            let proxy_for = |body: [i32; 3]| {
                let mut cells = scene.clone();
                cells.push((body, BODY_MATERIAL));
                proxy(BOX_ORIGIN, BOX_DIMENSIONS, &cells)
            };
            let mut measured = retained_volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
            measured.set_mesh_proxy(Some(proxy_for(path[0])));
            let (mut frames, mut rays, mut work) = complete(&mut measured, &world, SUN);
            let mut invalidated = 0usize;
            let mut retained = 0usize;
            let mut max_frames_per_move = 0usize;
            let mut landed = 0usize;
            // Intermediate references: exact equality must hold under motion,
            // not only at the end.
            for (step, &body) in path.iter().enumerate().take(61).skip(1) {
                let edit = measured.replace_mesh_proxy(Some(proxy_for(body)));
                assert!(edit.tracked, "{label} frame {step}");
                assert_eq!(edit.changed_cells, 2, "{label} frame {step}: one-cell move");
                invalidated += edit.invalidated_faces;
                retained += edit.retained_faces;
                let (move_frames, move_rays, move_work) = complete(&mut measured, &world, SUN);
                frames += move_frames;
                rays += move_rays;
                work += move_work;
                max_frames_per_move = max_frames_per_move.max(move_frames);
                if step % 10 == 0 {
                    let fresh = fresh_reference(
                        BOX_ORIGIN,
                        BOX_DIMENSIONS,
                        GATHER_DISTANCE_M,
                        &world,
                        SUN,
                        proxy_for(body),
                    );
                    assert_matches_fresh(
                        &measured,
                        &fresh,
                        BOX_ORIGIN,
                        BOX_DIMENSIONS,
                        &format!("{label} frame {step}"),
                    );
                    landed += 1;
                }
            }
            let fresh = fresh_reference(
                BOX_ORIGIN,
                BOX_DIMENSIONS,
                GATHER_DISTANCE_M,
                &world,
                SUN,
                proxy_for(path[60]),
            );
            assert_matches_fresh(
                &measured,
                &fresh,
                BOX_ORIGIN,
                BOX_DIMENSIONS,
                &format!("{label} final"),
            );

            // Whole-volume invalidation of the same 60 moves, for the cost
            // comparison only: same proxy sequence, same budget, no retention.
            let mut whole = volume(BOX_ORIGIN, BOX_DIMENSIONS, GATHER_DISTANCE_M);
            whole.set_mesh_proxy(Some(proxy_for(path[0])));
            let (mut whole_frames, mut whole_rays, mut whole_work) =
                complete(&mut whole, &world, SUN);
            for &body in path.iter().take(61).skip(1) {
                whole.set_mesh_proxy(Some(proxy_for(body)));
                let (move_frames, move_rays, move_work) = complete(&mut whole, &world, SUN);
                whole_frames += move_frames;
                whole_rays += move_rays;
                whole_work += move_work;
            }
            let total_slots = measured.values.len();
            println!(
                "[retention] {label}: moves=60 slots={total_slots} invalidated={invalidated} \
                 retained={retained} retained_fraction={:.3} frames={frames} rays={rays} work={work} \
                 max_frames_per_move={max_frames_per_move} intermediates={landed} \
                 whole_frames={whole_frames} whole_rays={whole_rays} whole_work={whole_work} \
                 ray_ratio={:.3} work_ratio={:.3}",
                retained as f64 / (retained + invalidated) as f64,
                rays as f64 / whole_rays as f64,
                work as f64 / whole_work as f64,
            );
            assert!(
                retained > 8 * invalidated,
                "{label}: exact tracking must retain most faces but kept {retained} of {}",
                retained + invalidated
            );
            assert!(
                rays < whole_rays && work < whole_work,
                "{label}: retention must cost less work than whole-volume invalidation: \
                 {rays}/{work} vs {whole_rays}/{whole_work}"
            );
        }
    }
}
