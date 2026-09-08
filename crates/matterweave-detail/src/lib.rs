//! Fine-scale authoritative detail volumes, prototypes and instances.
//!
//! This crate adds a *local* fine cell scale (metres per cell) on top of the
//! existing sparse [`matterweave_core::World`] and its block-mesh greedy mesher.
//! It never rescales the global world, physics, save format or material IDs:
//! every detail volume owns a scale and an explicit transform into world metres.
//!
//! Conventions
//! - Local cell coordinates are signed `[i32; 3]`; cell `c` occupies the local
//!   metre box `[c * scale, (c + 1) * scale]`.
//! - A volume's local origin is the corner of cell `[0, 0, 0]`.
//! - A [`Transform`] is a quarter-turn yaw about the local Y axis followed by a
//!   translation in world metres. The translation is an arbitrary finite,
//!   bounded `f32` per axis: fractional-metre placement is fully supported.
//!   Only *rotation* is restricted, to quarter turns.
//! - Transforms and queries are computed in `f32`. Rotation and translation of a
//!   quarter-turn transform introduce no rounding beyond ordinary `f32` addition,
//!   but a query point lying exactly on a cell boundary resolves according to
//!   `f32` rounding of that addition. No exactness is claimed for arbitrary
//!   floating-point inputs.
//!
//! Budgets
//! - Every allocation path is bounded before allocating: see [`MAX_VOLUME_CHUNKS`],
//!   [`MAX_SCENE_SOURCE_BYTES`], [`MAX_SCENE_CACHE_BYTES`], [`MAX_MESH_BYTES`] and
//!   [`MAX_SNAPSHOT_JSON_BYTES`]. These are conservative prototype budgets chosen
//!   to hold the current fixtures on a desktop host. They are not a claim about
//!   phone feasibility.

mod bracket;
pub use bracket::bracket_fungus;
mod fixtures;
mod flora;
mod scene;
mod select;
mod serial;
mod showcase;
mod wetland_flora;

pub use fixtures::{
    column_top, gallery_scene, parasol_mushroom, terrain_detail_tile, FIXTURE_GENERATOR_VERSION,
    MUSHROOM_FOOT_RADIUS_CELLS, MUSHROOM_SCALE, TILE_EDGE_CELLS, TILE_WATER_LEVEL_CELLS,
};
pub use flora::{
    assert_policy, clustered_mushroom, dense_tile, fan_frond, flora_class, flora_prototype,
    funnel_mushroom, instance_support, reed_cluster, rosette_groundcover, CORRIDOR_TILE_X,
    DENSE_FLORA_CELLS_MIN, DENSE_PER_SPECIES_MIN, DENSE_TYPES_MIN, DENSE_VEGETATION_MIN,
    FLORA_CANONICAL_SEED, FLORA_FUNGUS_SCALE_M, FLORA_GENERATOR_VERSION, FLORA_LEAF_SCALE_M,
    FLORA_SPECIES,
};
pub use scene::{
    DetailScene, InstanceDraw, SceneCounts, SceneVersion, MAX_INSTANCES, MAX_PROTOTYPES,
    MAX_SCENE_CACHE_BYTES, MAX_SCENE_SOURCE_BYTES, MAX_SCENE_TRANSLATION_M,
};
pub use select::{
    Camera, ErrorMetrics, InstanceLod, LodConfig, MeshBatch, PreparedFrame, Projection,
};
pub use serial::{VolumeSnapshot, MAX_SNAPSHOT_JSON_BYTES, MAX_SNAPSHOT_RUNS, SNAPSHOT_VERSION};
pub use showcase::{
    build_showcase, carved, cell_centre_m, composition_hash, showcase_class, showcase_prototype,
    species_source_radius_m, terrain_height_m, ClassCounts, ContentManifest, Landmark, Showcase,
    SurfacePoint, Terrain, BAND_CELLS, BASE_ROCK_ID, BASE_SOIL_ID, BASIN_CENTRE_M,
    BASIN_WATER_LEVEL_M, EYE_HEIGHT_M, LILY_PAD_HEIGHT_M, MAP_EDGE_CELLS, MAP_EDGE_M,
    MAX_PROTOTYPE_CELLS, MAX_TERRAIN_CELL_Y, ROUTE_PLAYER_CLEARANCE_M, ROUTE_STEP_M,
    SHOWCASE_EXPANDED_CELLS_MIN, SHOWCASE_FLORA_CELLS_MIN, SHOWCASE_GENERATOR_VERSION,
    SHOWCASE_PLANTS_MIN, SHOWCASE_SEED, SHOWCASE_SPECIES, SPECIES_LILY, TERRAIN_CELL_M,
    TILES_PER_EDGE, TILE_CELLS, WALK_SPEED_M_S,
};
pub use wetland_flora::{
    horsetail, marsh_lily, twisted_shrub, wetland_prototype, WETLAND_FLORA_SPECIES,
    WETLAND_LEAF_SCALE_M, WETLAND_SPIRE_SCALE_M,
};

use matterweave_core::{Mesh, Vertex, World, CHUNK_EDGE, CHUNK_VOLUME};

/// Fine detail scale used for flora prototypes: 6.25 cm cells.
pub const SCALE_FINE_M: f32 = 0.0625;
/// Terrain/detail tile scale: 25 cm cells.
pub const SCALE_TILE_M: f32 = 0.25;
/// Smallest accepted cell size in metres.
pub const MIN_SCALE_M: f32 = 0.001;
/// Largest accepted cell size in metres (one metre, the existing world cell size).
pub const MAX_SCALE_M: f32 = 1.0;
/// Inclusive bound on a local cell coordinate on any axis.
pub const MAX_CELL_COORD: i32 = 1 << 16;
/// Resident chunks per volume. 1024 chunks x 4 KiB = 4 MiB of source payload.
pub const MAX_VOLUME_CHUNKS: usize = 1024;
/// Occupied cells per volume, implied by [`MAX_VOLUME_CHUNKS`].
pub const MAX_VOLUME_CELLS: usize = MAX_VOLUME_CHUNKS * CHUNK_VOLUME;
/// Source payload bytes per volume.
pub const MAX_VOLUME_SOURCE_BYTES: usize = MAX_VOLUME_CHUNKS * CHUNK_VOLUME;
/// Vertex/index output bound for one derived mesh, checked before generation.
/// Core chunk mesher and bounded coarsening scratch are additional temporary data.
pub const MAX_MESH_BYTES: usize = 32 * 1024 * 1024;
/// Worst-case vertices a single occupied cell can contribute (six quads).
const MAX_VERTICES_PER_CELL: usize = 24;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailError {
    InvalidScale,
    CellOutOfRange,
    /// A metre-space point was NaN, infinite or outside the supported extent.
    InvalidPoint,
    DuplicatePrototype(String),
    DuplicateInstance(String),
    UnknownPrototype(String),
    InvalidTransform,
    SceneFull,
    /// A declared budget would be exceeded. Prior data and caches are unchanged.
    BudgetExceeded(&'static str),
    MalformedSnapshot(&'static str),
}

impl std::fmt::Display for DetailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScale => write!(f, "scale must be finite and within bounds"),
            Self::CellOutOfRange => write!(f, "cell coordinate out of supported range"),
            Self::InvalidPoint => write!(f, "point is not finite or out of supported range"),
            Self::DuplicatePrototype(id) => write!(f, "duplicate prototype id {id}"),
            Self::DuplicateInstance(id) => write!(f, "duplicate instance id {id}"),
            Self::UnknownPrototype(id) => write!(f, "unknown prototype id {id}"),
            Self::InvalidTransform => write!(f, "transform is not finite or out of range"),
            Self::SceneFull => write!(f, "scene capacity exceeded"),
            Self::BudgetExceeded(what) => write!(f, "budget exceeded: {what}"),
            Self::MalformedSnapshot(why) => write!(f, "malformed snapshot: {why}"),
        }
    }
}

impl std::error::Error for DetailError {}

pub type Result<T> = std::result::Result<T, DetailError>;

/// Validated positive, finite, bounded cell size in metres.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Scale(f32);

impl Scale {
    pub fn new(metres: f32) -> Result<Self> {
        if !metres.is_finite() || !(MIN_SCALE_M..=MAX_SCALE_M).contains(&metres) {
            return Err(DetailError::InvalidScale);
        }
        Ok(Self(metres))
    }
    pub fn metres(self) -> f32 {
        self.0
    }
}

/// Supported yaw values. Quarter turns keep axis-aligned faces axis aligned, so
/// derived normals and bounds stay axis aligned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Yaw {
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl Yaw {
    /// Right-handed rotation about +Y: `x' = x cos + z sin`, `z' = -x sin + z cos`.
    fn rotate(self, p: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = p;
        match self {
            Self::Deg0 => [x, y, z],
            Self::Deg90 => [z, y, -x],
            Self::Deg180 => [-x, y, -z],
            Self::Deg270 => [-z, y, x],
        }
    }
    fn inverse(self) -> Self {
        match self {
            Self::Deg0 => Self::Deg0,
            Self::Deg90 => Self::Deg270,
            Self::Deg180 => Self::Deg180,
            Self::Deg270 => Self::Deg90,
        }
    }
}

/// Yaw then translation, from local metres to world metres.
///
/// The fields are public for ergonomic construction, so a value can be built
/// without validation. Every public API that consumes a `Transform` revalidates
/// it with [`Transform::validate`] and returns [`DetailError::InvalidTransform`]
/// rather than producing geometry or query results from a bad transform.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transform {
    /// Arbitrary finite translation in world metres, including fractional values.
    pub translation_m: [f32; 3],
    pub yaw: Yaw,
}

impl Transform {
    pub fn new(translation_m: [f32; 3], yaw: Yaw) -> Result<Self> {
        let transform = Self { translation_m, yaw };
        transform.validate()?;
        Ok(transform)
    }
    pub fn identity() -> Self {
        Self {
            translation_m: [0.0; 3],
            yaw: Yaw::Deg0,
        }
    }
    pub fn is_valid(&self) -> bool {
        self.translation_m
            .iter()
            .all(|v| v.is_finite() && v.abs() <= MAX_SCENE_TRANSLATION_M)
    }
    pub fn validate(&self) -> Result<()> {
        if self.is_valid() {
            Ok(())
        } else {
            Err(DetailError::InvalidTransform)
        }
    }
    pub fn point_to_world(&self, local_m: [f32; 3]) -> [f32; 3] {
        let r = self.yaw.rotate(local_m);
        std::array::from_fn(|axis| r[axis] + self.translation_m[axis])
    }
    pub fn point_to_local(&self, world_m: [f32; 3]) -> [f32; 3] {
        let t = std::array::from_fn(|axis| world_m[axis] - self.translation_m[axis]);
        self.yaw.inverse().rotate(t)
    }
    pub fn direction_to_world(&self, local: [f32; 3]) -> [f32; 3] {
        self.yaw.rotate(local)
    }
}

/// Axis-aligned box in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    fn transformed(self, transform: &Transform) -> Self {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for corner in 0..8u8 {
            let p = std::array::from_fn(|axis| {
                if corner >> axis & 1 == 1 {
                    self.max[axis]
                } else {
                    self.min[axis]
                }
            });
            let w = transform.point_to_world(p);
            for axis in 0..3 {
                min[axis] = min[axis].min(w[axis]);
                max[axis] = max[axis].max(w[axis]);
            }
        }
        Self { min, max }
    }
}

/// How derived representations may use a material. Decorative materials never
/// enter collision; liquid is queryable as a volume but is not a physical wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialPolicy {
    Collision,
    Liquid,
    Decorative,
}

/// Named palette owned by this crate. IDs are local to detail volumes and do not
/// change or reuse any existing core world material definitions.
pub mod material {
    pub const AIR: u8 = 0;
    pub const DETAIL_SOIL: u8 = 10;
    pub const MOSS_TURF: u8 = 11;
    pub const BANK_STONE: u8 = 12;
    pub const WATER: u8 = 13;
    pub const MUSHROOM_STIPE: u8 = 20;
    pub const MUSHROOM_CAP: u8 = 21;
    pub const MUSHROOM_RIM: u8 = 22;
    pub const MUSHROOM_GILL: u8 = 23;
    // Additive flora catalogue palette (IDs 30..63). Existing IDs above are unchanged.
    pub const FLORA_FUNNEL_STIPE: u8 = 30;
    pub const FLORA_FUNNEL_CAP: u8 = 31;
    pub const FLORA_FUNNEL_RIM: u8 = 32;
    pub const FLORA_FUNNEL_GILL: u8 = 33;
    pub const FLORA_CLUSTER_STIPE: u8 = 34;
    pub const FLORA_CLUSTER_CAP: u8 = 35;
    pub const FLORA_CLUSTER_RIM: u8 = 36;
    pub const FLORA_CLUSTER_GILL: u8 = 37;
    pub const FLORA_FROND_STEM: u8 = 38;
    pub const FLORA_FROND_BLADE: u8 = 39;
    pub const FLORA_FROND_RIB: u8 = 40;
    pub const FLORA_REED_STEM: u8 = 41;
    pub const FLORA_REED_LEAF: u8 = 42;
    pub const FLORA_REED_PLUME: u8 = 43;
    pub const FLORA_ROSETTE_LEAF: u8 = 44;
    pub const FLORA_ROSETTE_HEART: u8 = 45;
    pub const FLORA_ROSETTE_SPOT: u8 = 46;
    pub const FLORA_LUMEN_DOT: u8 = 47;
    /// Luminous accent grown *in fungal flesh*. Separate ID from
    /// [`FLORA_LUMEN_DOT`] precisely because it is part of a substantive body
    /// and must collide; colour may match, policy may not.
    pub const FLORA_FUNGUS_LUMEN: u8 = 48;
}

pub fn material_name(material: u8) -> &'static str {
    match material {
        material::DETAIL_SOIL => "detail_soil",
        material::MOSS_TURF => "moss_turf",
        material::BANK_STONE => "bank_stone",
        material::WATER => "water",
        material::MUSHROOM_STIPE => "mushroom_stipe",
        material::MUSHROOM_CAP => "mushroom_cap",
        material::MUSHROOM_RIM => "mushroom_rim",
        material::MUSHROOM_GILL => "mushroom_gill",
        material::FLORA_FUNNEL_STIPE => "flora_funnel_stipe",
        material::FLORA_FUNNEL_CAP => "flora_funnel_cap",
        material::FLORA_FUNNEL_RIM => "flora_funnel_rim",
        material::FLORA_FUNNEL_GILL => "flora_funnel_gill",
        material::FLORA_CLUSTER_STIPE => "flora_cluster_stipe",
        material::FLORA_CLUSTER_CAP => "flora_cluster_cap",
        material::FLORA_CLUSTER_RIM => "flora_cluster_rim",
        material::FLORA_CLUSTER_GILL => "flora_cluster_gill",
        material::FLORA_FROND_STEM => "flora_frond_stem",
        material::FLORA_FROND_BLADE => "flora_frond_blade",
        material::FLORA_FROND_RIB => "flora_frond_rib",
        material::FLORA_REED_STEM => "flora_reed_stem",
        material::FLORA_REED_LEAF => "flora_reed_leaf",
        material::FLORA_REED_PLUME => "flora_reed_plume",
        material::FLORA_ROSETTE_LEAF => "flora_rosette_leaf",
        material::FLORA_ROSETTE_HEART => "flora_rosette_heart",
        material::FLORA_ROSETTE_SPOT => "flora_rosette_spot",
        material::FLORA_LUMEN_DOT => "flora_lumen_dot",
        material::FLORA_FUNGUS_LUMEN => "flora_fungus_lumen",
        _ => "unknown",
    }
}

/// Water is authoritative source data but is never a collision wall; gills and
/// rim are collidable parts of the same fungal body, matching its visual shape.
pub fn material_policy(material: u8) -> MaterialPolicy {
    match material {
        material::WATER => MaterialPolicy::Liquid,
        material::AIR => MaterialPolicy::Decorative,
        material::FLORA_FUNNEL_STIPE
        | material::FLORA_FUNNEL_CAP
        | material::FLORA_FUNNEL_RIM
        | material::FLORA_FUNNEL_GILL
        | material::FLORA_CLUSTER_STIPE
        | material::FLORA_CLUSTER_CAP
        | material::FLORA_CLUSTER_RIM
        | material::FLORA_CLUSTER_GILL
        | material::FLORA_FUNGUS_LUMEN => MaterialPolicy::Collision,
        material::FLORA_FROND_STEM
        | material::FLORA_FROND_BLADE
        | material::FLORA_FROND_RIB
        | material::FLORA_REED_STEM
        | material::FLORA_REED_LEAF
        | material::FLORA_REED_PLUME
        | material::FLORA_ROSETTE_LEAF
        | material::FLORA_ROSETTE_HEART
        | material::FLORA_ROSETTE_SPOT
        | material::FLORA_LUMEN_DOT => MaterialPolicy::Decorative,
        _ => MaterialPolicy::Collision,
    }
}

pub fn material_color(material: u8) -> [f32; 3] {
    match material {
        material::DETAIL_SOIL => [0.30, 0.23, 0.18],
        material::MOSS_TURF => [0.24, 0.42, 0.31],
        material::BANK_STONE => [0.44, 0.44, 0.45],
        material::WATER => [0.16, 0.34, 0.42],
        material::MUSHROOM_STIPE => [0.85, 0.82, 0.71],
        material::MUSHROOM_CAP => [0.72, 0.45, 0.22],
        material::MUSHROOM_RIM => [0.58, 0.33, 0.18],
        material::MUSHROOM_GILL => [0.91, 0.86, 0.78],
        material::FLORA_FUNNEL_STIPE => [0.80, 0.68, 0.47],
        material::FLORA_FUNNEL_CAP => [0.66, 0.42, 0.20],
        material::FLORA_FUNNEL_RIM => [0.50, 0.30, 0.16],
        material::FLORA_FUNNEL_GILL => [0.90, 0.84, 0.72],
        material::FLORA_CLUSTER_STIPE => [0.78, 0.74, 0.80],
        material::FLORA_CLUSTER_CAP => [0.45, 0.32, 0.55],
        material::FLORA_CLUSTER_RIM => [0.33, 0.22, 0.42],
        material::FLORA_CLUSTER_GILL => [0.85, 0.80, 0.88],
        material::FLORA_FROND_STEM => [0.25, 0.45, 0.40],
        material::FLORA_FROND_BLADE => [0.24, 0.52, 0.38],
        material::FLORA_FROND_RIB => [0.55, 0.75, 0.60],
        material::FLORA_REED_STEM => [0.35, 0.50, 0.30],
        material::FLORA_REED_LEAF => [0.30, 0.55, 0.33],
        material::FLORA_REED_PLUME => [0.80, 0.75, 0.60],
        material::FLORA_ROSETTE_LEAF => [0.28, 0.50, 0.36],
        material::FLORA_ROSETTE_HEART => [0.45, 0.68, 0.40],
        material::FLORA_ROSETTE_SPOT => [0.75, 0.55, 0.25],
        material::FLORA_LUMEN_DOT => [0.65, 0.85, 0.80],
        material::FLORA_FUNGUS_LUMEN => [0.62, 0.88, 0.76],
        _ => [0.60, 0.60, 0.60],
    }
}

/// Derived level of detail. `Source` is the authoritative resolution; coarser
/// levels are derived copies and never replace or mutate the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lod {
    Source,
    Half,
    Quarter,
}

impl Lod {
    pub fn factor(self) -> i32 {
        match self {
            Self::Source => 1,
            Self::Half => 2,
            Self::Quarter => 4,
        }
    }
}

/// Authoritative sparse editable detail volume at one fine scale.
///
/// Storage and greedy meshing reuse [`matterweave_core::World`]; this type owns
/// identity, scale, validation, budgets, palette mapping and derived-LOD policy.
#[derive(Clone)]
pub struct DetailVolume {
    id: String,
    scale: Scale,
    world: World,
    /// Maintained incrementally; `set` never rescans the whole volume.
    occupied: usize,
    chunks: usize,
}

fn chunk_key(cell: [i32; 3]) -> [i32; 3] {
    cell.map(|v| v.div_euclid(CHUNK_EDGE))
}

impl DetailVolume {
    pub fn new(id: impl Into<String>, scale: Scale) -> Self {
        Self {
            id: id.into(),
            scale,
            world: World::new(0),
            occupied: 0,
            chunks: 0,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn scale(&self) -> Scale {
        self.scale
    }
    /// Monotonic content revision. Derived caches compare against this value.
    pub fn revision(&self) -> u64 {
        self.world.revision()
    }
    /// O(1): maintained by [`DetailVolume::set`], not by scanning chunks.
    pub fn occupied_cells(&self) -> usize {
        self.occupied
    }
    pub fn chunk_count(&self) -> usize {
        self.chunks
    }
    /// Authoritative payload bytes only: resident chunk arrays. Excludes map
    /// overhead, derived meshes, LOD copies and instance bookkeeping.
    pub fn source_bytes(&self) -> usize {
        self.chunks * CHUNK_VOLUME
    }

    /// Signed range membership; never uses `abs`, which would panic or wrap at
    /// `i32::MIN`.
    fn check_cell(cell: [i32; 3]) -> Result<()> {
        if cell
            .iter()
            .all(|v| (-MAX_CELL_COORD..=MAX_CELL_COORD).contains(v))
        {
            Ok(())
        } else {
            Err(DetailError::CellOutOfRange)
        }
    }

    pub fn get(&self, cell: [i32; 3]) -> u8 {
        if Self::check_cell(cell).is_err() {
            return material::AIR;
        }
        self.world.get(cell)
    }

    /// Edits the source. Returns whether material data changed.
    ///
    /// Fails with [`DetailError::BudgetExceeded`] *before* allocating a new
    /// chunk that would exceed [`MAX_VOLUME_CHUNKS`]; existing data is unchanged.
    pub fn set(&mut self, cell: [i32; 3], material: u8) -> Result<bool> {
        Self::check_cell(cell)?;
        let previous = self.world.get(cell);
        if previous == material {
            return Ok(false);
        }
        let key = chunk_key(cell);
        let had_chunk = self.world.chunk_revision(key).is_some();
        if !had_chunk && material != material::AIR && self.chunks >= MAX_VOLUME_CHUNKS {
            return Err(DetailError::BudgetExceeded("volume chunk budget"));
        }
        if !self.world.set(cell, material) {
            return Ok(false);
        }
        if previous == material::AIR {
            self.occupied += 1;
        } else if material == material::AIR {
            self.occupied -= 1;
        }
        let has_chunk = self.world.chunk_revision(key).is_some();
        match (had_chunk, has_chunk) {
            (false, true) => self.chunks += 1,
            (true, false) => self.chunks -= 1,
            _ => {}
        }
        Ok(true)
    }

    /// Occupied cells in stable order: chunk key, then z, y, x.
    pub fn iter_cells(&self) -> impl Iterator<Item = ([i32; 3], u8)> + '_ {
        self.world.chunk_keys().into_iter().flat_map(move |key| {
            (0..16).flat_map(move |z| {
                (0..16).flat_map(move |y| {
                    (0..16).filter_map(move |x| {
                        let cell = [key[0] * 16 + x, key[1] * 16 + y, key[2] * 16 + z];
                        match self.world.get(cell) {
                            material::AIR => None,
                            m => Some((cell, m)),
                        }
                    })
                })
            })
        })
    }

    /// Occupied cell bounding box in local cell units, inclusive of `max`.
    pub fn cell_bounds(&self) -> Option<([i32; 3], [i32; 3])> {
        let mut min = [i32::MAX; 3];
        let mut max = [i32::MIN; 3];
        for (cell, _) in self.iter_cells() {
            for axis in 0..3 {
                min[axis] = min[axis].min(cell[axis]);
                max[axis] = max[axis].max(cell[axis]);
            }
        }
        (min[0] <= max[0]).then_some((min, max))
    }

    /// Local-metre bounds of occupied cells.
    pub fn bounds_local(&self) -> Option<Bounds> {
        let (min, max) = self.cell_bounds()?;
        let s = self.scale.metres();
        Some(Bounds {
            min: min.map(|v| v as f32 * s),
            max: max.map(|v| (v + 1) as f32 * s),
        })
    }

    /// World-metre bounds under `transform`. `Ok(None)` means the volume is empty.
    pub fn bounds_world(&self, transform: &Transform) -> Result<Option<Bounds>> {
        transform.validate()?;
        Ok(self.bounds_local().map(|b| b.transformed(transform)))
    }

    /// Cell containing a point given in local metres. Rejects NaN, infinities and
    /// points outside the supported cell range instead of silently clamping.
    pub fn cell_at_local_metres(&self, point: [f32; 3]) -> Result<[i32; 3]> {
        let mut cell = [0i32; 3];
        for axis in 0..3 {
            let value = point[axis];
            if !value.is_finite() {
                return Err(DetailError::InvalidPoint);
            }
            let index = (value / self.scale.metres()).floor();
            if !(index >= -(MAX_CELL_COORD as f32) && index <= MAX_CELL_COORD as f32) {
                return Err(DetailError::InvalidPoint);
            }
            cell[axis] = index as i32;
        }
        Ok(cell)
    }

    /// Material at a local-metre point.
    pub fn sample_local_metres(&self, point: [f32; 3]) -> Result<u8> {
        Ok(self.get(self.cell_at_local_metres(point)?))
    }

    /// Material at a world-metre point under `transform`. Invalid transforms and
    /// non-finite or out-of-range points are rejected, never treated as a hit.
    pub fn sample_world_metres(&self, transform: &Transform, point: [f32; 3]) -> Result<u8> {
        transform.validate()?;
        if !point.iter().all(|v| v.is_finite()) {
            return Err(DetailError::InvalidPoint);
        }
        self.sample_local_metres(transform.point_to_local(point))
    }

    pub(crate) fn mesh_upper_bound_bytes(&self) -> Result<usize> {
        let bytes_per_cell =
            MAX_VERTICES_PER_CELL * std::mem::size_of::<Vertex>() + 36 * std::mem::size_of::<u32>();
        let upper_bound = self.occupied.saturating_mul(bytes_per_cell);
        if upper_bound > MAX_MESH_BYTES {
            return Err(DetailError::BudgetExceeded("derived mesh output budget"));
        }
        Ok(upper_bound)
    }

    /// Derived greedy surface mesh in local metres, using the crate palette.
    /// Chunk-seam shared faces are removed by the core halo mesher.
    ///
    /// The worst-case output size is checked against [`MAX_MESH_BYTES`] before
    /// any geometry is generated, using 24 vertices and 36 indices per occupied cell.
    pub fn mesh_local(&self) -> Result<Mesh> {
        self.mesh_upper_bound_bytes()?;
        let mut out = Mesh {
            revision: self.revision(),
            ..Mesh::default()
        };
        let s = self.scale.metres();
        for key in self.world.chunk_keys() {
            let chunk = self.world.mesh_chunk(key);
            // Reserve exactly the appended chunk, avoiding geometric spare capacity.
            out.vertices.reserve_exact(chunk.vertices.len());
            out.indices.reserve_exact(chunk.indices.len());
            let base = u32::try_from(out.vertices.len()).expect("mesh exceeds u32 index range");
            // Core emits four vertices per greedy quad, so one palette lookup per
            // quad is enough: step half a cell inward from the quad centroid to
            // land in the owning solid cell rather than the air cell outside.
            for quad in chunk.vertices.chunks(4) {
                let normal = quad[0].normal;
                let centroid: [f32; 3] = std::array::from_fn(|axis| {
                    quad.iter().map(|v| v.position[axis]).sum::<f32>() / quad.len() as f32
                });
                let inside: [i32; 3] = std::array::from_fn(|axis| {
                    (centroid[axis] - 0.5 * normal[axis]).floor() as i32
                });
                let color = material_color(self.world.get(inside));
                for vertex in quad {
                    out.vertices.push(Vertex {
                        position: vertex.position.map(|v| v * s),
                        normal,
                        color,
                    });
                }
            }
            out.indices
                .extend(chunk.indices.into_iter().map(|i| i + base));
        }
        Ok(out)
    }

    /// Derived mesh in world metres under `transform`. Winding is preserved
    /// because only proper rotations and uniform positive scale are applied.
    pub fn mesh_world(&self, transform: &Transform) -> Result<Mesh> {
        transform.validate()?;
        let mut mesh = self.mesh_local()?;
        for vertex in &mut mesh.vertices {
            vertex.position = transform.point_to_world(vertex.position);
            vertex.normal = transform.direction_to_world(vertex.normal);
        }
        Ok(mesh)
    }

    /// Derived coarser copy at `scale * factor`. The source volume is never
    /// modified and the result is not authoritative.
    ///
    /// Returns [`DetailError::InvalidScale`] when the derived scale would leave
    /// the supported range; the scale is never clamped, because clamping would
    /// silently shrink the derived geometry relative to the source.
    /// A coarse cell is occupied when any source cell inside it is occupied, so
    /// coarse bounds may conservatively expand by up to one coarse cell per face.
    pub fn coarsen(&self, lod: Lod) -> Result<DetailVolume> {
        let factor = lod.factor();
        let scale = Scale::new(self.scale.metres() * factor as f32)?;
        // Conservatively bound vote entries before allocating the coarse scratch map.
        self.mesh_upper_bound_bytes()?;
        let mut out = DetailVolume::new(format!("{}#lod{factor}", self.id), scale);
        if factor == 1 {
            out.world = self.world.clone();
            out.occupied = self.occupied;
            out.chunks = self.chunks;
            return Ok(out);
        }
        // Compact per-coarse-cell votes: detail volumes use a handful of
        // materials, so a short association list costs a few bytes per cell
        // instead of a 1 KiB dense table.
        let mut votes: std::collections::BTreeMap<[i32; 3], Vec<(u8, u32)>> =
            std::collections::BTreeMap::new();
        for (cell, material) in self.iter_cells() {
            let coarse = cell.map(|v| v.div_euclid(factor));
            let entry = votes.entry(coarse).or_default();
            match entry.iter_mut().find(|(id, _)| *id == material) {
                Some((_, count)) => *count += 1,
                None => entry.push((material, 1)),
            }
        }
        for (cell, counts) in votes {
            // Majority material; ties resolve to the lowest ID for determinism.
            let material = counts
                .iter()
                .copied()
                .max_by_key(|&(id, count)| (count, std::cmp::Reverse(id)))
                .expect("non-empty vote list")
                .0;
            out.set(cell, material)?;
        }
        Ok(out)
    }
}
