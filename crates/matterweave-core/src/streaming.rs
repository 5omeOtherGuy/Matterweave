use crate::{address, hash, Chunk, World, CHUNK_EDGE, CHUNK_VOLUME};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const STREAM_RADIUS_CHUNKS: i32 = 3;
pub const WORLD_LIMIT: i32 = 256;
/// Vertical band of the original island fixture's window.
pub const STREAM_MIN_Y: i32 = -16;
pub const STREAM_MAX_Y: i32 = 32;
/// Vertical band of the landscape generator: it produces surfaces from
/// [`crate::landscape::MIN_SURFACE_Y`] to [`crate::landscape::MAX_SURFACE_Y`],
/// so the window has to cover that span plus one chunk of headroom above peaks.
pub const LANDSCAPE_MIN_Y: i32 = -48;
pub const LANDSCAPE_MAX_Y: i32 = 160;
pub(crate) const MAX_OVERRIDES: usize = 512;

#[derive(Clone)]
pub(crate) struct Streaming {
    // None is an explicitly empty edited chunk, never absence of an override.
    pub overrides: BTreeMap<[i32; 3], Option<Chunk>>,
    pub resident: BTreeSet<[i32; 3]>,
    pub center: Option<[i32; 2]>,
}

impl World {
    /// Opt in to the versioned terrain extension. The original square remains completely
    /// snapshot-authoritative: missing legacy chunks are air, including deletions.
    /// Saved chunks elsewhere are retained as overrides, including outside bounds.
    pub fn enable_streaming(&mut self) {
        if self.streaming.is_none() {
            self.streaming = Some(Streaming {
                overrides: self
                    .chunks
                    .iter()
                    .map(|(&key, value)| (key, Some(value.clone())))
                    .collect(),
                resident: BTreeSet::new(),
                center: None,
            });
        }
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming.is_some()
    }

    /// Legal editable simulation domain. Streaming additionally requires residency.
    pub fn contains_stream_cell(&self, cell: [i32; 3]) -> bool {
        self.terrain.contains_cell(cell)
    }

    /// Window center for an eye position, or `None` for nonfinite input.
    /// One definition serves the synchronous call and background preparation.
    pub(crate) fn stream_center_of(position: [f32; 3]) -> Option<[i32; 2]> {
        position.iter().all(|v| v.is_finite()).then(|| {
            [position[0], position[2]].map(|v| ((v.floor() as i32).div_euclid(16)).clamp(-16, 15))
        })
    }

    /// Published residency including empty chunks; None means a non-streaming
    /// world whose data is all authoritative in memory. Bounded to 147 keys.
    pub fn stream_resident_chunks(&self) -> Option<Vec<[i32; 3]>> {
        self.streaming
            .as_ref()
            .map(|s| s.resident.iter().copied().collect())
    }

    /// Center of the currently published window, if streaming has published one.
    pub(crate) fn stream_center(&self) -> Option<[i32; 2]> {
        self.streaming.as_ref().and_then(|stream| stream.center)
    }

    /// Whether every chunk overlapping `position` inflated by `margin` is resident,
    /// including resident chunks that hold no material. Residency is what makes
    /// movement and collision safe; `chunk_keys` only lists nonempty chunks.
    /// A world without streaming is entirely authoritative in memory, so any
    /// finite in-bounds position is contained. Nonfinite or oversized margins,
    /// positions outside the simulation domain and unpublished windows are false.
    pub fn stream_contains_position(&self, position: [f32; 3], margin: f32) -> bool {
        if !position.iter().all(|v| v.is_finite()) || !(0.0..=64.0).contains(&margin) {
            return false;
        }
        let low = position.map(|v| (f64::from(v) - f64::from(margin)).floor());
        let high = position.map(|v| (f64::from(v) + f64::from(margin)).floor());
        let (min_y, max_y) = self.stream_y_range();
        if !low
            .iter()
            .zip(high)
            .enumerate()
            .all(|(axis, (&low, high))| {
                let (min, max) = if axis == 1 {
                    (f64::from(min_y), f64::from(max_y - 1))
                } else {
                    (f64::from(-WORLD_LIMIT), f64::from(WORLD_LIMIT - 1))
                };
                low >= min && high <= max
            })
        {
            return false;
        }
        let Some(stream) = &self.streaming else {
            return true;
        };
        let chunk = |cell: [f64; 3]| cell.map(|v| (v as i32).div_euclid(CHUNK_EDGE));
        let (low, high) = (chunk(low), chunk(high));
        for x in low[0]..=high[0] {
            for y in low[1]..=high[1] {
                for z in low[2]..=high[2] {
                    if !stream.resident.contains(&[x, y, z]) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Synchronously publish a bounded `7 x 7` window centered on `position`,
    /// covering the terrain source's vertical band. No stale background jobs
    /// exist; callers synchronize render/collision revisions before advancing
    /// simulation. At revision exhaustion or with nonfinite input residency is
    /// left unchanged.
    pub fn stream_around(&mut self, position: [f32; 3]) -> bool {
        let Some(center) = Self::stream_center_of(position) else {
            return false;
        };
        let Some(stream) = &self.streaming else {
            return false;
        };
        if stream.center == Some(center) {
            return false;
        }
        let Some(revision) = self.revision.checked_add(1) else {
            return false;
        };
        let mut wanted = BTreeSet::new();
        for x in
            (center[0] - STREAM_RADIUS_CHUNKS).max(-16)..=(center[0] + STREAM_RADIUS_CHUNKS).min(15)
        {
            for z in (center[1] - STREAM_RADIUS_CHUNKS).max(-16)
                ..=(center[1] + STREAM_RADIUS_CHUNKS).min(15)
            {
                for y in self.stream_y_chunks() {
                    wanted.insert([x, y, z]);
                }
            }
        }
        let mut changed: BTreeSet<_> = stream
            .resident
            .symmetric_difference(&wanted)
            .copied()
            .collect();
        // First publication also evicts legacy chunks outside the new window.
        changed.extend(
            self.chunks
                .keys()
                .filter(|key| !wanted.contains(*key))
                .copied(),
        );
        self.chunks.retain(|key, _| wanted.contains(key));
        self.chunk_revisions.retain(|key, _| wanted.contains(key));
        let terrain = self.terrain;
        let seed = self.seed;
        let mut missing = BTreeSet::new();
        for &key in wanted.difference(&stream.resident) {
            if let Some(value) = stream.overrides.get(&key) {
                if let Some(chunk) = value {
                    self.chunks.insert(key, chunk.clone());
                }
            } else if !(terrain == crate::TerrainSource::LegacyIsland
                && (-2..2).contains(&key[0])
                && (-2..2).contains(&key[2]))
            {
                missing.insert(key);
            }
        }
        for (key, chunk) in generated_chunks(seed, terrain, &missing, self.stream_y_chunks()) {
            self.chunks.insert(key, chunk);
        }
        self.revision = revision;
        for key in changed {
            for (axis, offset) in [(0, 0), (0, -1), (0, 1), (1, -1), (1, 1), (2, -1), (2, 1)] {
                let mut neighbor = key;
                neighbor[axis] += offset;
                if self.chunks.contains_key(&neighbor) {
                    self.chunk_revisions.insert(neighbor, revision);
                }
            }
        }
        let stream = self.streaming.as_mut().unwrap();
        stream.resident = wanted;
        stream.center = Some(center);
        true
    }
}

pub(crate) fn generated_chunk(
    seed: u64,
    source: crate::TerrainSource,
    key: [i32; 3],
) -> Option<Chunk> {
    match source {
        crate::TerrainSource::LegacyIsland => legacy_chunk(seed, key),
        crate::TerrainSource::Landscape => {
            let mut missing = BTreeSet::new();
            missing.insert(key);
            landscape_stacks(seed, &missing, key[1]..key[1] + 1)
                .into_iter()
                .map(|(_, chunk)| chunk)
                .next()
        }
    }
}

/// Generate every requested chunk that is not already stored.
///
/// The landscape source groups requests by their `(x, z)` chunk column and
/// samples each metre-column once for the whole vertical band, then fills every
/// requested layer from that one sample. Sampling per chunk instead would repeat
/// the whole noise stack once per 16 m layer. The legacy fixture is a low, shallow
/// island whose window has three layers, so per-chunk generation stays correct
/// and cheap for it.
fn generated_chunks(
    seed: u64,
    source: crate::TerrainSource,
    missing: &BTreeSet<[i32; 3]>,
    y_chunks: std::ops::Range<i32>,
) -> Vec<([i32; 3], Chunk)> {
    match source {
        crate::TerrainSource::LegacyIsland => missing
            .iter()
            .filter_map(|&key| legacy_chunk(seed, key).map(|chunk| (key, chunk)))
            .collect(),
        crate::TerrainSource::Landscape => landscape_stacks(seed, missing, y_chunks),
    }
}

fn landscape_stacks(
    seed: u64,
    missing: &BTreeSet<[i32; 3]>,
    y_chunks: std::ops::Range<i32>,
) -> Vec<([i32; 3], Chunk)> {
    use crate::landscape;
    let layers_supported = y_chunks.len();
    let mut by_column: BTreeMap<[i32; 2], Vec<i32>> = BTreeMap::new();
    for key in missing {
        by_column.entry([key[0], key[2]]).or_default().push(key[1]);
    }
    let mut generated = Vec::new();
    for ([chunk_x, chunk_z], mut layers) in by_column {
        layers.sort_unstable();
        debug_assert!(layers.len() <= layers_supported);
        let mut voxels = vec![[0u8; CHUNK_VOLUME]; layers.len()];
        let mut solid = vec![0usize; layers.len()];
        for x in chunk_x * 16..chunk_x * 16 + 16 {
            for z in chunk_z * 16..chunk_z * 16 + 16 {
                let column = landscape::column(seed, x, z);
                for (slot, &chunk_y) in layers.iter().enumerate() {
                    let bottom = chunk_y * 16;
                    let floor = bottom.max(LANDSCAPE_MIN_Y);
                    let mut y = (bottom + 15).min(column.height);
                    while y >= floor {
                        let material = landscape::material_in_column(seed, &column, x, y, z);
                        if material != 0 {
                            voxels[slot][address([x, y, z]).1] = material;
                            solid[slot] += 1;
                        }
                        y -= 1;
                    }
                }
            }
        }
        for (slot, payload) in voxels.into_iter().enumerate() {
            if solid[slot] == 0 {
                continue;
            }
            generated.push((
                [chunk_x, layers[slot], chunk_z],
                Chunk {
                    voxels: Arc::new(payload),
                    solid: solid[slot],
                },
            ));
        }
    }
    generated
}

fn legacy_chunk(seed: u64, key: [i32; 3]) -> Option<Chunk> {
    let mut chunk = Chunk {
        voxels: Arc::new([0; CHUNK_VOLUME]),
        solid: 0,
    };
    let voxels = Arc::make_mut(&mut chunk.voxels);
    for x in key[0] * 16..key[0] * 16 + 16 {
        for z in key[2] * 16..key[2] * 16 + 16 {
            // Match the closest legacy edge falloff (positive edge 31, negative
            // edge -32), then recover gradually across the surrounding basin.
            let cx = x.div_euclid(8);
            let cz = z.div_euclid(8);
            let fx = x.rem_euclid(8);
            let fz = z.rem_euclid(8);
            let noise = |dx, dz| (hash(seed, cx + dx, cz + dz) % 7) as i32;
            let a = noise(0, 0) * (8 - fx) + noise(1, 0) * fx;
            let b = noise(0, 1) * (8 - fx) + noise(1, 1) * fx;
            let edge_x = x.clamp(-32, 31);
            let edge_z = z.clamp(-32, 31);
            let edge_falloff = (edge_x.abs().max(edge_z.abs()) - 20).max(0) / 3;
            let distance = (x - edge_x).abs().max((z - edge_z).abs());
            let falloff = (edge_falloff - distance / 4).max(0);
            let height = (a * (8 - fz) + b * fz) / 64 - falloff;
            for y in key[1] * 16..key[1] * 16 + 16 {
                if y < -8 || y > height {
                    continue;
                }
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
                voxels[address([x, y, z]).1] = material;
                chunk.solid += 1;
            }
        }
    }
    (chunk.solid != 0).then_some(chunk)
}
