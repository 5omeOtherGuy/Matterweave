//! Versioned procedural landscape: biome terrain, relief, water levels and flora.
//!
//! This is a pure, deterministic function of `(seed, coordinate)`. It allocates
//! nothing on the sampling path, reads no world state and has no platform
//! dependency, so the same seed and generator version produce byte-identical
//! columns on every target. All arithmetic is integer or fixed point; no
//! `f32`/`f64` operation participates in a generated column, material, flora site
//! or distance sample. The derived tile mesh is render output and does use floats
//! for its vertices and normals; no float value flows back into a generated
//! result. That is deliberate: a stored world must regenerate exactly what it was
//! saved from, and ARM64 and x86-64 must not drift.
//!
//! # Fields
//!
//! Coordinates are world metres (one voxel is one metre on a side, as in
//! [`crate`]). Four bounded noise fields compose the terrain:
//!
//! - **continent** (512 m lattice): deep ocean through high plateau,
//! - **hills** (128 m lattice): local relief,
//! - **ridge** (320 m lattice, ridged): crest lines, masked by the continent field,
//! - **temperature** (1 km lattice) and **humidity** (640 m lattice): biome selection.
//!
//! Sea level is [`SEA_LEVEL`] and generated terrain stays inside
//! [`MIN_SURFACE_Y`]`..=`[`MAX_SURFACE_Y`].
//!
//! # Water
//!
//! Water is *not* stored in the world. A column whose surface lies below
//! [`SEA_LEVEL`] reports [`Column::water_level`], and the derived render paths
//! (the near-field surface mesh and the far tile mesh) place the surface. The
//! authoritative world therefore stays free of liquid voxels: collision, saves
//! and edits are unchanged, and no lake has to be filled with cells. Editing
//! terrain below sea level does not move the water surface in this version; that
//! is a stated limitation rather than a silent behaviour.
//!
//! # Distance levels
//!
//! [`column`] samples one metre. [`lod_tile_mesh`] builds a derived surface tile
//! from the same generator at `1 << level` metre cells, for the renderer's nested
//! distance rings. Each coarse cell is flat at its own [`lod_step_m`]-quantised
//! height sampled at the cell centre, and each difference to a neighbouring cell
//! is a vertical wall spanning the full step, so a tile reads as voxel blocks
//! rather than as a smoothed heightfield. Planes of equal height are
//! greedy-merged, and a face between two cells belongs to the higher one, so two
//! tiles meeting at a border agree on the wall between them instead of drawing it
//! twice. Relief narrower than a coarse cell is not represented at that level;
//! [`lod_sample`] provides the conservative maximum for tools and silhouette
//! checks.

use crate::hash;
use crate::material;
use crate::mesh::{Mesh, Vertex};
use crate::{CHUNK_EDGE, STREAM_RADIUS_CHUNKS};
use std::collections::HashMap;

/// Identity of this generator's output. Bump for any change to the columns,
/// materials or flora population it produces.
///
/// Version 2: ground cover and flowers are placed at the pressures and bands the
/// vegetation pass ships (denser grass, sparse dune grass, coastal palms), so
/// the flora population is not the one version 1 produced. Columns, materials
/// and tile meshes are unchanged.
///
/// Version 3: the ground-cover density is a continuous profile in distance
/// (`density_percent` per band) instead of an integer decimation step, so the
/// population at every distance past the first band differs from version 2's.
/// Columns, materials, tree placement and tile meshes are unchanged.
pub const LANDSCAPE_GENERATOR_VERSION: u32 = 3;

/// Sea surface height in metres. Columns strictly below this are flooded.
pub const SEA_LEVEL: i32 = 0;
/// Deepest surface the generator produces.
pub const MIN_SURFACE_Y: i32 = -40;
/// Highest surface the generator produces.
pub const MAX_SURFACE_Y: i32 = 150;

/// Coarse cells per tile edge produced by [`lod_tile_mesh`].
pub const LOD_TILE_CELLS: i32 = 32;
/// Highest supported [`lod_sample`]/[`lod_tile_mesh`] level: cell is `1 << level` m.
pub const MAX_LOD_LEVEL: u32 = 6;
/// Tree population lattice in metres. Larger than [`FLORA_CELL_M`] because a tree
/// is a landmark rather than ground cover.
pub const TREE_CELL_M: i32 = 8;

/// Flora population lattice in metres: one cell holds up to [`MAX_FLORA_PER_CELL`] sites.
pub const FLORA_CELL_M: i32 = 2;
/// Flora slots sampled per cell.
pub const MAX_FLORA_PER_CELL: usize = 12;

/// Sentinel for a column that carries no water surface.
pub const NO_WATER: i32 = i32::MIN;

/// Distinct landscape regions selected from the climate and relief fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Biome {
    Ocean,
    Beach,
    Desert,
    Plains,
    Forest,
    Swamp,
    Hills,
    Mountain,
    Snow,
    Tundra,
}

impl Biome {
    /// Every biome, for coverage checks, manifests and authoring tools.
    pub const ALL: [Biome; 10] = [
        Biome::Ocean,
        Biome::Beach,
        Biome::Desert,
        Biome::Plains,
        Biome::Forest,
        Biome::Swamp,
        Biome::Hills,
        Biome::Mountain,
        Biome::Snow,
        Biome::Tundra,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Biome::Ocean => "ocean",
            Biome::Beach => "beach",
            Biome::Desert => "desert",
            Biome::Plains => "plains",
            Biome::Forest => "forest",
            Biome::Swamp => "swamp",
            Biome::Hills => "hills",
            Biome::Mountain => "mountain",
            Biome::Snow => "snow",
            Biome::Tundra => "tundra",
        }
    }

    /// Grass tuft pressure in `0..=64` per flora slot; 64 fills every slot.
    ///
    /// Ground cover is the layer a close capture judges the landscape by, so the
    /// grassy biomes sit near the top of the range: the slot is one clump per
    /// metre column, and one clump is a clump of thin blades, not a mat. Beach
    /// and desert keep a sparse dune-grass cover rather than none, which is what
    /// the reference shore shows.
    pub fn grass_density(self) -> u8 {
        match self {
            Biome::Ocean => 0,
            Biome::Snow => 2,
            Biome::Desert => 3,
            Biome::Beach => 8,
            Biome::Mountain => 14,
            Biome::Tundra => 16,
            Biome::Swamp => 48,
            Biome::Hills => 56,
            Biome::Forest => 62,
            Biome::Plains => 64,
        }
    }

    /// Second, independent ground-cover pressure in `0..=64` per flora slot.
    ///
    /// One clump per metre column still leaves the ground showing between
    /// clumps at the grazing angle a walking eye sees it from, because a clump
    /// is a clump of thin blades and not a mat. This slot rolls its own hash, so
    /// a second clump can stand in the same metre column: the pair is one denser
    /// tuft rather than two clumps apart, which is what closes the gaps without
    /// a prototype the size of the gap. It is 0 where ground cover is meant to
    /// be thin, so the sparse biomes keep their single sparse layer.
    pub fn understory_density(self) -> u8 {
        match self {
            Biome::Ocean | Biome::Snow | Biome::Desert => 0,
            Biome::Beach => 4,
            Biome::Mountain => 6,
            Biome::Tundra => 8,
            Biome::Swamp => 40,
            Biome::Hills => 48,
            Biome::Forest => 56,
            Biome::Plains => 64,
        }
    }

    /// Flower pressure in `0..=64` per flora slot.
    ///
    /// The flower layer stays a minority of the field - a meadow with a flower
    /// on every column would be a flower bed - but it is a distinct layer rather
    /// than a garnish, and it is densest where the reference meadow is.
    pub fn flower_density(self) -> u8 {
        match self {
            Biome::Ocean | Biome::Snow => 0,
            Biome::Desert => 4,
            Biome::Beach => 5,
            Biome::Mountain => 8,
            Biome::Tundra => 10,
            Biome::Swamp => 12,
            Biome::Hills => 16,
            Biome::Forest => 16,
            Biome::Plains => 26,
        }
    }

    /// Tree pressure in `0..=64` per 8 m cell.
    ///
    /// Beach carries a sparse palm layer (the reference's sand is treed, not
    /// bare), and the coastal margin is where a spawn camera looks first, so the
    /// shoreline is not an empty strip between the water and the forest.
    pub fn tree_density(self) -> u8 {
        match self {
            Biome::Forest => 44,
            Biome::Swamp => 22,
            Biome::Plains | Biome::Hills => 6,
            Biome::Beach => 3,
            Biome::Tundra => 3,
            Biome::Desert | Biome::Mountain | Biome::Snow | Biome::Ocean => 0,
        }
    }

    /// Whether exposed ground is grass rather than bare rock, sand or snow.
    pub fn grassy(self) -> bool {
        matches!(
            self,
            Biome::Plains | Biome::Forest | Biome::Hills | Biome::Swamp | Biome::Tundra
        )
    }
}

/// One metre-column of the generated surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    /// Topmost solid surface cell.
    pub height: i32,
    /// Water surface when the column is flooded, [`NO_WATER`] otherwise.
    pub water_level: i32,
    pub biome: Biome,
    /// Material of the surface cell.
    pub surface: u8,
    /// Material between the surface and deep rock.
    pub sub_surface: u8,
    /// Thickness of the sub-surface layer in metres.
    pub sub_depth: i32,
}

impl Column {
    pub fn flooded(&self) -> bool {
        self.water_level != NO_WATER
    }

    /// Depth of water above the ground in metres, zero when not flooded.
    pub fn water_depth(&self) -> i32 {
        if self.flooded() {
            self.water_level - self.height
        } else {
            0
        }
    }
}

/// A plant kind the landscape population requests. Prototype geometry and scale
/// live in the detail layer; this enum is only the placement vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloraKind {
    GrassTuft,
    FlowerRed,
    FlowerWhite,
    FlowerYellow,
    Fern,
    Shrub,
    TreeBroadleaf,
    TreeConifer,
    Reed,
    Cactus,
}

impl FloraKind {
    pub fn name(self) -> &'static str {
        match self {
            FloraKind::GrassTuft => "grass_tuft",
            FloraKind::FlowerRed => "flower_red",
            FloraKind::FlowerWhite => "flower_white",
            FloraKind::FlowerYellow => "flower_yellow",
            FloraKind::Fern => "fern",
            FloraKind::Shrub => "shrub",
            FloraKind::TreeBroadleaf => "tree_broadleaf",
            FloraKind::TreeConifer => "tree_conifer",
            FloraKind::Reed => "reed",
            FloraKind::Cactus => "cactus",
        }
    }
}

/// One deterministic plant placement on the metre grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloraSite {
    pub kind: FloraKind,
    pub x: i32,
    /// Ground surface cell the plant stands on.
    pub y: i32,
    pub z: i32,
    /// Quarter turns about Y, `0..=3`, matching the detail/instance yaw encoding.
    pub yaw_quarters: u8,
    /// Scale in eighths, `4..=12` (0.5x to 1.5x).
    pub scale_eighths: u8,
}

/// Bounded population of one flora cell. Unused slots repeat the last site.
#[derive(Clone, Copy, Debug)]
pub struct FloraCell {
    count: u8,
    sites: [FloraSite; MAX_FLORA_PER_CELL],
}

impl FloraCell {
    /// The populated sites, in deterministic order.
    pub fn sites(&self) -> &[FloraSite] {
        &self.sites[..self.count as usize]
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// One derive-only sample for distance rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LodSample {
    /// Terrain surface in metres; [`SEA_LEVEL`] when flooded.
    pub height: i32,
    /// Surface material at that height; [`material::WATER`] when flooded.
    pub surface: u8,
    /// Biome of the sampled column, [`Biome::Ocean`] when flooded.
    pub biome: Biome,
    /// True when every sampled column is flooded.
    pub flooded: bool,
}

impl Default for LodSample {
    fn default() -> Self {
        Self {
            height: SEA_LEVEL,
            surface: material::WATER,
            biome: Biome::Ocean,
            flooded: true,
        }
    }
}

/// Rectangular clip region in world metres, edges aligned to the level's cell
/// grid. [`lod_tile_mesh`] omits cells whose centre lies inside and adds a skirt
/// along the resulting boundary, which is how a finer ring's square is cut out of
/// a coarser ring without overlap and without a gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Clip {
    pub min: [i32; 2],
    pub max: [i32; 2],
}

impl Clip {
    /// Whether the centre of an aligned cell falls inside the clip.
    pub fn contains_centre(&self, centre: [i32; 2]) -> bool {
        centre[0] >= self.min[0]
            && centre[0] < self.max[0]
            && centre[1] >= self.min[1]
            && centre[1] < self.max[1]
    }
}

/// What portion of the distance grid one tile mesh covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TileFilter {
    /// Cells inside this square are omitted; a finer level covers them.
    pub hole: Option<Clip>,
    /// Cells outside this square are omitted; a coarser level covers them.
    pub bound: Option<Clip>,
    /// Flooded cells inside this square are drawn as the ground under them
    /// instead of as the sea surface, because a finer water surface covers
    /// them.
    ///
    /// This is the difference between a *hole* and a *bed*. A ring that
    /// surrounds the authoritative streaming window must draw that window's
    /// ground, or the camera stands in clear colour until the fine meshes
    /// arrive - but it must not draw a second sea surface there: the derived
    /// tile's water is opaque and a metre of water colour over the bed would
    /// hide the bed the fine translucent surface is meant to show through. So
    /// its flooded cells become the bed at their own quantised height, which
    /// the fine bed (a metre above) and the fine water both cover.
    pub bed: Option<Clip>,
}

impl TileFilter {
    /// Whether the coarse cell of `level` that contains world metre point
    /// `(x, z)` is drawn by this filter.
    ///
    /// The test is the mesher's own: a cell is drawn when its centre lies in
    /// the bound and not in the hole. It says nothing about which tile owns the
    /// cell, so a caller holding a tile must use [`RingTile::covers_point`],
    /// which asks this of the filter only once the tile owns the cell.
    pub fn covers_point(&self, level: u32, x: i32, z: i32) -> bool {
        let cell_m = lod_cell_m(level);
        let centre = [
            x.div_euclid(cell_m) * cell_m + cell_m / 2,
            z.div_euclid(cell_m) * cell_m + cell_m / 2,
        ];
        let in_hole = self.hole.is_some_and(|hole| hole.contains_centre(centre));
        let in_bound = self.bound.is_none_or(|bound| bound.contains_centre(centre));
        !in_hole && in_bound
    }
}

// -- Fixed-point noise -------------------------------------------------------

const SALT_CONTINENT: u64 = 0x9e37_79b9_7f4a_7c15;
const SALT_HILLS: u64 = 0xc2b2_ae3d_27d4_eb4f;
const SALT_RIDGE: u64 = 0x1656_67b1_9e37_79f9;
const SALT_TEMP: u64 = 0x27d4_eb2f_1656_67c1;
const SALT_HUMID: u64 = 0x85eb_ca6b_c2b2_ae35;
const SALT_FLORA: u64 = 0x1656_67b1_9e37_79f3;
const SALT_ORE: u64 = 0x2545_f491_4f6c_dd1d;
const SALT_ROCK: u64 = 0x9e37_79b9_7f4a_7c0f;

/// Deterministic three-input mixing for slot and site selection. `std`'s hashers
/// are not a stable cross-release contract, so the project's own mixing is used.
fn hash3(seed: u64, x: i32, y: i32) -> u64 {
    let mut value = seed
        ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (y as u64).wrapping_mul(0xd6e8_feb8_6659_fd93);
    value = (value ^ (value >> 32)).wrapping_mul(0xd6e8_feb8_6659_fd93);
    value = (value ^ (value >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^ (value >> 32)
}

/// Deterministic four-input mixing for voxel-level detail (ore and deep rock),
/// where folding one coordinate into another would repeat patterns along a diagonal.
/// Also the mixer behind [`crate::material::tone`], which is why it is visible
/// to the rest of the crate.
pub(crate) fn hash4(seed: u64, x: i32, y: i32, z: i32) -> u64 {
    let mut value = seed
        ^ (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
        ^ (z as u64).wrapping_mul(0xd6e8_feb8_6659_fd93);
    value = (value ^ (value >> 32)).wrapping_mul(0xd6e8_feb8_6659_fd93);
    value = (value ^ (value >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^ (value >> 32)
}

/// Lattice value in `0..=65535`.
fn lattice(seed: u64, salt: u64, x: i32, z: i32) -> i32 {
    ((hash(seed ^ salt, x, z) >> 24) as i32) & 0xFFFF
}

/// Bilinear value noise on a `cell`-metre lattice, in `0..=65535`. The fraction
/// is quantised to 1/256 of a cell, so the field is an exact integer function of
/// the coordinate for every cell size.
fn value_noise(seed: u64, salt: u64, x: i32, z: i32, cell: i32) -> i32 {
    let cell = cell.max(1);
    let lx = x.div_euclid(cell);
    let lz = z.div_euclid(cell);
    let fx = (x.rem_euclid(cell) as i64 * 256) / cell as i64;
    let fz = (z.rem_euclid(cell) as i64 * 256) / cell as i64;
    let c00 = lattice(seed, salt, lx, lz) as i64;
    let c10 = lattice(seed, salt, lx + 1, lz) as i64;
    let c01 = lattice(seed, salt, lx, lz + 1) as i64;
    let c11 = lattice(seed, salt, lx + 1, lz + 1) as i64;
    let a = c00 * (256 - fx) + c10 * fx;
    let b = c01 * (256 - fx) + c11 * fx;
    ((a * (256 - fz) + b * fz) >> 16) as i32
}

fn octave_salt(salt: u64, octave: u32) -> u64 {
    salt.wrapping_add((octave as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// Octave-summed value noise in `0..=65535`. Weights halve per octave and the
/// lattice halves with them, so the result stays inside the base range.
fn fbm(seed: u64, salt: u64, x: i32, z: i32, first_cell: i32, octaves: u32) -> i32 {
    let mut sum = 0i64;
    let mut weight = 0i64;
    let mut w = 256i64;
    let mut cell = first_cell.max(1);
    for octave in 0..octaves {
        let value = value_noise(seed, octave_salt(salt, octave), x, z, cell) as i64;
        sum += value * w;
        weight += w;
        w >>= 1;
        cell = (cell >> 1).max(1);
    }
    if weight == 0 {
        0
    } else {
        (sum / weight) as i32
    }
}

/// Ridged noise in `0..=65535`: each octave is `1 - |2v - 1|`, which peaks
/// mid-lattice and produces crest lines rather than rolling hills.
fn ridged(seed: u64, salt: u64, x: i32, z: i32, first_cell: i32, octaves: u32) -> i32 {
    let mut sum = 0i64;
    let mut weight = 0i64;
    let mut w = 256i64;
    let mut cell = first_cell.max(1);
    for octave in 0..octaves {
        let value = value_noise(seed, octave_salt(salt, octave), x, z, cell) as i64;
        let ridge = (65535 - 2 * (value - 32768).abs()).max(0);
        sum += ridge * w;
        weight += w;
        w >>= 1;
        cell = (cell >> 1).max(1);
    }
    if weight == 0 {
        0
    } else {
        (sum / weight) as i32
    }
}

/// Integer ramp from `0` at `low` to `256` at `high`, clamped.
fn ramp(value: i32, low: i32, high: i32) -> i32 {
    let span = (high - low).max(1);
    (((value - low).max(0) as i64 * 256) / span as i64).min(256) as i32
}

// -- Column sampling ---------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Fields {
    continent: i32,
    hills: i32,
    ridge: i32,
    temperature: i32,
    humidity: i32,
    /// Signed base elevation before hills and mountains.
    elevation: i32,
    /// Mountain contribution in metres.
    mountain: i32,
}

fn fields(seed: u64, x: i32, z: i32) -> Fields {
    let continent = fbm(seed, SALT_CONTINENT, x, z, 512, 5);
    let hills = fbm(seed, SALT_HILLS, x, z, 128, 3);
    let ridge = ridged(seed, SALT_RIDGE, x, z, 320, 4);
    let temperature = fbm(seed, SALT_TEMP, x, z, 1024, 3);
    let humidity = fbm(seed, SALT_HUMID, x, z, 640, 3);
    let signed = continent - 32768;
    // Ocean floor descends to MIN_SURFACE_Y; land rises to a plateau and
    // mountains are added on top where the continent field is high enough.
    let elevation = if signed < 0 {
        -4 + (signed as i64 * 36 / 32768) as i32
    } else {
        2 + (signed as i64 * 48 / 32768) as i32
    };
    let masked = ramp(continent, 38000, 49000);
    // ridge and masked are both fixed point, so the product carries two units of
    // fraction beyond metres: divide by 65536 * 256 (shift 24) to scale the ridge
    // to the 0..111 m mountain range.
    let mountain = ((ridge as i64 * masked as i64 * 112) >> 24) as i32;
    Fields {
        continent,
        hills,
        ridge,
        temperature,
        humidity,
        elevation,
        mountain,
    }
}

fn hill_relief(fields: &Fields) -> i32 {
    let hill = ((fields.hills - 32768) as i64 * 10 / 32768) as i32;
    // Ocean relief is softened so basins stay basins.
    if fields.continent < 32000 {
        hill / 3
    } else {
        hill
    }
}

fn biome_of(fields: &Fields, height: i32) -> Biome {
    if height < SEA_LEVEL {
        return Biome::Ocean;
    }
    let cold = fields.temperature < 26000;
    let hot = fields.temperature > 41000;
    let wet = fields.humidity > 43000;
    let dry = fields.humidity < 26000;
    // Snow line rises with temperature: about 109 m in the coldest regions and
    // 129 m in the warmest, never below 72 m.
    let snow_line = (118 + ((fields.temperature - 32768) as i64 * 46 / 32768) as i32).max(72);
    if height >= snow_line {
        return Biome::Snow;
    }
    if height <= SEA_LEVEL + 1 {
        return if wet || fields.humidity > 36000 {
            Biome::Swamp
        } else {
            Biome::Beach
        };
    }
    if hot && dry {
        return Biome::Desert;
    }
    if cold {
        return Biome::Tundra;
    }
    if height > 74 {
        return Biome::Mountain;
    }
    if wet {
        return Biome::Forest;
    }
    if hill_relief(fields).abs() > 5 {
        return Biome::Hills;
    }
    Biome::Plains
}

fn surface_materials(biome: Biome, fields: &Fields) -> (u8, u8, i32) {
    match biome {
        Biome::Ocean => {
            if fields.elevation > -10 {
                (material::SAND, material::SAND, 3)
            } else {
                (material::GRAVEL, material::STONE, 2)
            }
        }
        Biome::Beach => (material::SAND, material::SAND, 4),
        Biome::Desert => {
            if fields.elevation > 30 {
                (material::STONE, material::CLAY, 2)
            } else {
                (material::SAND, material::CLAY, 3)
            }
        }
        Biome::Plains => (material::MOSS, material::SOIL, 3),
        Biome::Forest => (material::MOSS, material::SOIL, 4),
        Biome::Swamp => (material::MOSS, material::CLAY, 2),
        Biome::Hills => {
            if fields.ridge > 48000 {
                (material::STONE, material::GRAVEL, 1)
            } else {
                (material::MOSS, material::SOIL, 3)
            }
        }
        Biome::Mountain => (material::STONE, material::GRAVEL, 2),
        Biome::Snow => (material::SNOW, material::STONE, 1),
        Biome::Tundra => (material::SNOW, material::SOIL, 1),
    }
}

/// Sample one metre-column of the landscape.
pub fn column(seed: u64, x: i32, z: i32) -> Column {
    let fields = fields(seed, x, z);
    let raw = fields.elevation + hill_relief(&fields) + fields.mountain;
    let height = raw.clamp(MIN_SURFACE_Y, MAX_SURFACE_Y);
    let biome = biome_of(&fields, height);
    let (surface, sub_surface, sub_depth) = surface_materials(biome, &fields);
    let water_level = if height < SEA_LEVEL {
        SEA_LEVEL
    } else {
        NO_WATER
    };
    Column {
        height,
        water_level,
        biome,
        surface,
        sub_surface,
        sub_depth,
    }
}

/// Biome at a surface column; equal to [`column`]`(seed, x, z).biome`.
pub fn biome_at(seed: u64, x: i32, z: i32) -> Biome {
    column(seed, x, z).biome
}

/// Surface height in metres; equal to [`column`]`(seed, x, z).height`.
pub fn height_at(seed: u64, x: i32, z: i32) -> i32 {
    column(seed, x, z).height
}

/// Material of one voxel, for terrain generation. Water is never returned: the
/// surface is derived, not stored.
pub fn material_at(seed: u64, x: i32, y: i32, z: i32) -> u8 {
    material_in_column(seed, &column(seed, x, z), x, y, z)
}

/// Material of one voxel for an already-sampled column.
///
/// Filling a whole chunk column calls this instead of [`material_at`] so the
/// noise fields are evaluated once per metre-column rather than once per voxel.
/// The column must be the one [`column`] returns for `(x, z)`.
pub fn material_in_column(seed: u64, column: &Column, x: i32, y: i32, z: i32) -> u8 {
    if y > column.height {
        return material::AIR;
    }
    if y == column.height {
        return column.surface;
    }
    if y >= column.height - column.sub_depth {
        // Ore pockets are sampled in three dimensions, so a pocket stays a pocket
        // instead of becoming a full-height column of ore at one (x, z).
        if (column.sub_surface == material::STONE || column.sub_surface == material::GRAVEL)
            && ((hash4(seed ^ SALT_ORE, x, y, z) >> 40) & 0x3F) < 2
        {
            return material::MINERAL;
        }
        return column.sub_surface;
    }
    // Deep rock keeps a deterministic vein pattern so deep cuts are not flat.
    if (hash4(seed ^ SALT_ROCK, x, y, z) >> 44) & 0x3F < 3 {
        material::MINERAL
    } else {
        material::STONE
    }
}

/// Sample one flora cell's population. Placement is deterministic; the caller
/// owns prototype selection, budgets and instancing.
pub fn flora_cell(seed: u64, cell_x: i32, cell_z: i32) -> FloraCell {
    let origin_x = cell_x * FLORA_CELL_M;
    let origin_z = cell_z * FLORA_CELL_M;
    let fallback = FloraSite {
        kind: FloraKind::GrassTuft,
        x: origin_x,
        y: 0,
        z: origin_z,
        yaw_quarters: 0,
        scale_eighths: 8,
    };
    let mut cell = FloraCell {
        count: 0,
        sites: [fallback; MAX_FLORA_PER_CELL],
    };
    let columns = [
        (origin_x, origin_z),
        (origin_x + 1, origin_z),
        (origin_x, origin_z + 1),
        (origin_x + 1, origin_z + 1),
    ];
    for &(x, z) in &columns {
        if cell.len() >= MAX_FLORA_PER_CELL {
            break;
        }
        let column = column(seed, x, z);
        if column.flooded() {
            continue;
        }
        let mut push = |kind: FloraKind, roll: u64| {
            if cell.len() >= MAX_FLORA_PER_CELL {
                return;
            }
            cell.sites[cell.len()] = FloraSite {
                kind,
                x,
                y: column.height,
                z,
                yaw_quarters: ((roll >> 12) & 3) as u8,
                scale_eighths: (4 + ((roll >> 16) % 9)) as u8,
            };
            cell.count += 1;
        };
        let grass_roll = hash3(seed ^ SALT_FLORA, x, z);
        let grass = column.biome.grass_density() as i32;
        if grass > 0 && ((grass_roll & 0xFFFF) as i32) < grass * 1024 {
            let kind = match (grass_roll >> 24) & 0x3F {
                0..=1 if matches!(column.biome, Biome::Desert | Biome::Mountain) => {
                    FloraKind::Cactus
                }
                2..=5 if column.biome.grassy() => FloraKind::Fern,
                6 if column.biome == Biome::Swamp => FloraKind::Reed,
                7..=9 if column.biome == Biome::Tundra => FloraKind::Shrub,
                _ => FloraKind::GrassTuft,
            };
            push(kind, grass_roll >> 32);
        }
        // A second ground-cover slot on its own roll, so a metre column can
        // carry two clumps and the pair reads as one denser tuft. It uses the
        // understory pressure, which is zero where cover is meant to be thin.
        let understory_roll = hash3(seed ^ SALT_FLORA ^ 0x2f11_9a3d, x, z);
        let understory = column.biome.understory_density() as i32;
        if understory > 0 && ((understory_roll & 0xFFFF) as i32) < understory * 1024 {
            let kind = match (understory_roll >> 26) & 0x3F {
                0..=2 if column.biome.grassy() => FloraKind::Fern,
                3 if column.biome == Biome::Swamp => FloraKind::Reed,
                _ => FloraKind::GrassTuft,
            };
            push(kind, understory_roll >> 40);
        }
        // Flowers use an independent roll so a tuft and a flower can share a
        // column instead of displacing each other.
        let flower_roll = hash3(seed ^ SALT_FLORA ^ 0x51ed_2701, x, z);
        let flowers = column.biome.flower_density() as i32;
        if flowers > 0 && ((flower_roll & 0xFFFF) as i32) < flowers * 1024 {
            let kind = match (flower_roll >> 34) & 3 {
                0 => FloraKind::FlowerRed,
                1 => FloraKind::FlowerWhite,
                2 => FloraKind::FlowerYellow,
                _ => FloraKind::Fern,
            };
            push(kind, flower_roll >> 40);
        }
    }
    cell
}

/// Tree population for one [`TREE_CELL_M`]-metre cell.
///
/// Trees are sampled separately from [`flora_cell`] because they are sparse
/// landmarks on their own lattice rather than ground cover: one cell yields at
/// most one tree, and the cell's biome decides whether it exists at all.
pub fn tree_cell(seed: u64, cell_x: i32, cell_z: i32) -> Option<FloraSite> {
    let x =
        cell_x * TREE_CELL_M + (hash3(seed ^ SALT_FLORA ^ 0x1f83_d9ab, cell_x, cell_z) & 7) as i32;
    let z =
        cell_z * TREE_CELL_M + (hash3(seed ^ SALT_FLORA ^ 0x9e37_79b9, cell_x, cell_z) & 7) as i32;
    let column = column(seed, x, z);
    if column.flooded() || column.biome.tree_density() == 0 {
        return None;
    }
    let roll = hash3(seed ^ SALT_FLORA ^ 0xc2b2_ae3d, cell_x, cell_z);
    if (roll & 0xFFFF) as i32 >= column.biome.tree_density() as i32 * 1024 {
        return None;
    }
    // Cold or high ground grows conifers; warmer ground grows broadleaf.
    let cold = column.biome == Biome::Tundra || column.biome == Biome::Snow || column.height > 70;
    Some(FloraSite {
        kind: if cold {
            FloraKind::TreeConifer
        } else {
            FloraKind::TreeBroadleaf
        },
        x,
        y: column.height,
        z,
        yaw_quarters: ((roll >> 12) & 3) as u8,
        scale_eighths: (4 + ((roll >> 16) % 9)) as u8,
    })
}

// -- Flora placement planning ------------------------------------------------

/// One distance band of the flora placement plan.
///
/// `radius_m` is a Chebyshev (square) half-extent around the eye, not a circle:
/// the flora lattice is a square grid and a square band is an exact integer
/// predicate on the lattice coordinate, so a site never changes band because a
/// float distance landed either side of a rounding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloraTier {
    pub radius_m: i32,
    /// Ground-cover density at this band's outer edge, in percent of full. The
    /// density ramps smoothly from the previous band's edge to this one, so the
    /// boundary is not a step; the first band is full density and the last
    /// reaches zero, which is where ground cover ends.
    pub density_percent: u8,
}

/// The shipped density profile: full density to 32 m, thinning smoothly to
/// nothing at 48 m.
///
/// Ground cover is drawn where it resolves: a blade is one fine voxel (6.25 cm)
/// across, and the landscape render target resolves about 1.6 mrad per pixel
/// (65 degrees over 720 rows), so a blade is sub-pixel beyond ~40 m and a clump
/// is a handful of pixels. The profile therefore ends near that distance, but
/// it *fades* to it: an earlier profile kept every cell to 28 m and one in eight
/// to 40 m and then stopped, which is a visible fence at walking distance, and
/// because the plan is anchored to a lattice the fence moved with the player.
///
/// The first radius is a density *plateau*, not just an LOD band. A plan is
/// rebuilt only after the eye has left the committed anchor by the sample's
/// rebuild hysteresis (20 m), so the profile trails the eye by up to that
/// distance; the plateau is what keeps full density in front of the eye
/// whatever the trail is, and 32 m leaves at least a dozen metres of it. The
/// fade then has to be long enough not to read as a fence and short enough to
/// fit the instance budget: 16 m is six times the slope the replaced
/// `keep_every` step had, and the 48 m end costs nothing that 40 m did not -
/// the window is a square, so its area and the plan's cost are set by the
/// plateau, not by the last metres of the fade. Density is a percentage at each
/// band's outer edge and ramps smoothly from the previous edge, so no boundary
/// is a step. Radii ascend, are whole numbers of [`FLORA_CELL_M`] cells and are
/// the square half-extent the plan covers.
pub const LANDSCAPE_FLORA_TIERS: [FloraTier; 2] = [
    FloraTier {
        radius_m: 32,
        density_percent: 100,
    },
    FloraTier {
        radius_m: 48,
        density_percent: 0,
    },
];

/// Chebyshev half-extent in metres inside which [`plan_flora`] populates trees.
///
/// Trees are landmarks, not ground cover, so they reach far past the outermost
/// [`LANDSCAPE_FLORA_TIERS`] band: at 160 m a [`Biome::Forest`] cell yields
/// about 0.69 trees per [`TREE_CELL_M`] cell over 1600 cells, so a forest is
/// still closing in around the eye at the edge of the plan rather than thinning
/// into scattered props. The cost is one [`column`] sample per tree cell, which
/// is why this radius is documented rather than tuned per frame.
pub const LANDSCAPE_TREE_RADIUS_M: i32 = 160;

/// One planned plant: a [`FloraSite`] plus the band it was kept by and the
/// per-site variation the renderer needs to make it an individual.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacedFlora {
    pub kind: FloraKind,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub yaw_quarters: u8,
    pub scale_eighths: u8,
    /// Per-site height multiplier in percent, `FLORA_MIN_VARIATION_PERCENT
    /// ..=FLORA_MAX_VARIATION_PERCENT` (0.75x to 1.25x). Two plants in the same
    /// size tier are then not the same height, which is what stops a field from
    /// reading as one prototype repeated.
    pub height_percent: u8,
    /// Per-site wind compliance multiplier in percent, same range. Independent
    /// of [`Self::height_percent`]: stiffness is not a copy of tallness.
    pub bend_percent: u8,
    /// Index into the tier slice the caller passed; trees use the band their
    /// distance falls in, clamped to the last tier.
    pub tier: u8,
}

/// Lower bound of the per-site variation range, as a percentage.
pub const FLORA_MIN_VARIATION_PERCENT: u8 = 75;
/// Upper bound of the per-site variation range, as a percentage.
pub const FLORA_MAX_VARIATION_PERCENT: u8 = 125;

/// One variation field from its own hash of the site's column: a percentage in
/// the inclusive range `FLORA_MIN_VARIATION_PERCENT..=FLORA_MAX_VARIATION_PERCENT`.
/// Integer throughout: the same column returns the same value on every target,
/// and the modulo only ever folds the hash into the 51-value range.
fn varied_percent(seed: u64, salt: u64, x: i32, z: i32) -> u8 {
    let span = u64::from(FLORA_MAX_VARIATION_PERCENT - FLORA_MIN_VARIATION_PERCENT) + 1;
    (u64::from(FLORA_MIN_VARIATION_PERCENT) + hash3(seed ^ salt, x, z) % span) as u8
}

impl PlacedFlora {
    fn from_site(site: FloraSite, tier: u8, seed: u64) -> Self {
        Self {
            kind: site.kind,
            x: site.x,
            y: site.y,
            z: site.z,
            yaw_quarters: site.yaw_quarters,
            scale_eighths: site.scale_eighths,
            height_percent: varied_percent(seed, SALT_FLORA_HEIGHT, site.x, site.z),
            bend_percent: varied_percent(seed, SALT_FLORA_BEND, site.x, site.z),
            tier,
        }
    }
}

/// A bounded, deterministic placement plan for one eye position.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FloraPlan {
    pub sites: Vec<PlacedFlora>,
    pub trees: Vec<PlacedFlora>,
    /// Sites and trees the caps refused. Reported instead of silently
    /// truncating, so a caller can show placement pressure rather than
    /// wondering why a field stops at an invisible line.
    pub dropped: usize,
}

impl FloraPlan {
    /// Clear the plan without releasing its buffers.
    pub fn clear(&mut self) {
        self.sites.clear();
        self.trees.clear();
        self.dropped = 0;
    }
}

/// Chebyshev distance in metres from the eye column.
fn chebyshev(x: i32, z: i32, eye_x: i32, eye_z: i32) -> i32 {
    (x - eye_x).abs().max((z - eye_z).abs())
}

/// Index of the first tier whose radius covers `distance`, or `None` beyond the
/// outermost tier.
fn tier_of(tiers: &[FloraTier], distance: i32) -> Option<u8> {
    tiers
        .iter()
        .position(|tier| distance <= tier.radius_m)
        .map(|index| index as u8)
}

/// Ground-cover density in percent at Chebyshev `distance` from the eye, from
/// the tier profile.
///
/// The profile is flat at the first tier's density up to its radius, then a
/// smoothstep ramp to each following tier's density at its radius, and the last
/// tier's density beyond it. `t` is scaled to 0..=1000 so the whole ramp is
/// integer arithmetic on every target, and the smoothstep
/// `t*t*(3000-2t)/1e6` has zero slope at both ends, so neither edge of a band
/// is a visible step in the field. Radii are assumed ascending, which is what
/// [`tier_of`] needs too.
fn density_percent_at(tiers: &[FloraTier], distance: i32) -> u32 {
    let Some(first) = tiers.first() else {
        return 0;
    };
    if distance <= first.radius_m {
        return u32::from(first.density_percent);
    }
    let mut previous = first;
    for tier in &tiers[1..] {
        if distance <= tier.radius_m {
            let span = i64::from((tier.radius_m - previous.radius_m).max(1));
            let t = ((i64::from(distance) - i64::from(previous.radius_m)).max(0) * 1000 / span)
                .min(1000);
            let eased = t * t * (3000 - 2 * t) / 1_000_000;
            let from = i64::from(previous.density_percent);
            let to = i64::from(tier.density_percent);
            return (from + (to - from) * eased / 1000).clamp(0, 100) as u32;
        }
        previous = tier;
    }
    u32::from(previous.density_percent)
}

const SALT_FLORA_KEEP: u64 = 0x7f4a_7c15_bf58_476d;

/// Per-site height and bend variation salts. Each field hashes its own salt and
/// its own column coordinate, so a site's height and its wind compliance vary
/// independently: a tall tuft is as likely to be the stiff one as the flexible
/// one, and neither follows the yaw or the size tier (which come from the
/// site's own roll in [`flora_cell`]).
const SALT_FLORA_HEIGHT: u64 = 0x94d0_49bb_1331_11eb;
const SALT_FLORA_BEND: u64 = 0xd2b7_4405_3f4a_6a1d;

/// Whether a flora lattice cell survives at `density_percent`.
///
/// Survival is decided from the *lattice cell*, before [`flora_cell`] samples
/// any column, because the columns are the whole cost: a dense window out to
/// 52 m would sample 2809 cells (11 236 [`column`] evaluations) where a thinned
/// one samples about 2 000. Per-site survival cannot save that work, since a
/// site only exists once its column has been generated. The consequence is
/// stated rather than hidden: the field thins in whole 2 m cells, not individual
/// plants.
///
/// The threshold is a deterministic hash of the cell, so a cell survives with
/// exactly the requested probability and the same cell survives the same way on
/// every target and in every plan that covers it. A cell that is thinned out is
/// one hash, not a column sample.
fn keeps_cell(seed: u64, cell_x: i32, cell_z: i32, density_percent: u32) -> bool {
    if density_percent >= 100 {
        return true;
    }
    if density_percent == 0 {
        return false;
    }
    (hash3(seed ^ SALT_FLORA_KEEP, cell_x, cell_z) % 100) < u64::from(density_percent)
}

/// Whether the plan keeps the ground-cover lattice cell at `distance_m` from the
/// eye: the planner's own density predicate, without sampling the cell's
/// columns.
///
/// Exposed so a test or a coverage tool can measure the density profile itself.
/// A caller that wants the *plants* must use [`plan_flora`]; this answers only
/// whether that plan would visit the cell.
pub fn keeps_flora_cell(seed: u64, cell: [i32; 2], tiers: &[FloraTier], distance_m: i32) -> bool {
    keeps_cell(
        seed,
        cell[0],
        cell[1],
        density_percent_at(tiers, distance_m),
    )
}

/// Plan the flora around one eye position.
///
/// See [`plan_flora_into`]; this allocates a fresh plan and is meant for tests
/// and tools, not a frame loop.
pub fn plan_flora(
    seed: u64,
    eye: [f32; 3],
    tiers: &[FloraTier],
    max_sites: usize,
    max_trees: usize,
) -> FloraPlan {
    let mut plan = FloraPlan::default();
    plan_flora_into(seed, eye, tiers, max_sites, max_trees, &mut plan);
    plan
}

/// [`plan_flora`] writing into a caller-owned plan.
///
/// The plan's vectors are cleared, not reallocated, so a frame loop that keeps
/// one plan allocates nothing after its buffers have grown once.
///
/// Ground cover comes from [`flora_cell`] on its [`FLORA_CELL_M`] lattice,
/// inside the outermost tier's square. A cell belongs to the first tier whose
/// radius covers the Chebyshev distance from the eye to the cell origin, and is
/// kept with the probability [`density_percent_at`] gives that distance; no cell
/// is visited twice, so no site appears twice. Trees come from [`tree_cell`] on
/// its [`TREE_CELL_M`] lattice inside [`LANDSCAPE_TREE_RADIUS_M`]. Iteration is
/// row-major in ascending lattice order, so the same eye and tiers always
/// produce the same list.
///
/// Every placement also carries the per-site variation of its column:
/// [`PlacedFlora::height_percent`] and [`PlacedFlora::bend_percent`], each from
/// its own salt (see [`SALT_FLORA_HEIGHT`]) and therefore independent of the
/// yaw, the size tier and of each other. The fields are part of the plan, so the
/// determinism this function guarantees covers them too.
///
/// A non-finite eye coordinate folds to zero exactly as [`ring_plan`] folds it.
pub fn plan_flora_into(
    seed: u64,
    eye: [f32; 3],
    tiers: &[FloraTier],
    max_sites: usize,
    max_trees: usize,
    plan: &mut FloraPlan,
) {
    let mut cache = FloraCellCache::default();
    if !plan_flora_cached_into(
        seed,
        eye,
        tiers,
        max_sites,
        max_trees,
        usize::MAX,
        &mut cache,
        plan,
    ) {
        debug_assert!(false, "an unlimited sample budget must complete the plan");
    }
}

/// Bounded memo of generated flora populations.
///
/// [`flora_cell`] and [`tree_cell`] are pure functions of the seed and the
/// lattice coordinate, and a plan's cost is almost entirely the [`column`]
/// samples those functions take. A memo lets a field that moves reuse every
/// cell the previous window covered and pay only for the cells the new window
/// adds. [`plan_flora_cached_into`] trims the memo back to the window after a
/// completed plan, so a camera that never stops cannot grow it.
///
/// `samples` counts the misses, i.e. the cells actually generated. It is the
/// number a test compares against a from-scratch plan to show the memo is doing
/// the work, and the number a caller slices on.
#[derive(Clone, Debug, Default)]
pub struct FloraCellCache {
    ground: HashMap<[i32; 2], FloraCell>,
    trees: HashMap<[i32; 2], Option<FloraSite>>,
    samples: u64,
}

impl FloraCellCache {
    /// Ground-cover cells currently memoised.
    pub fn ground_cells(&self) -> usize {
        self.ground.len()
    }

    /// Tree cells currently memoised, including cells that hold no tree.
    pub fn tree_cells(&self) -> usize {
        self.trees.len()
    }

    /// Cells generated since construction, i.e. memo misses.
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Drop every memoised cell.
    pub fn clear(&mut self) {
        self.ground.clear();
        self.trees.clear();
    }

    /// Make room for `ground_cells` populations and `tree_cells` cells.
    ///
    /// A first plan fills a whole window, and every cell it inserts carries a
    /// [`FloraCell`] of up to [`MAX_FLORA_PER_CELL`] sites; reserving the window
    /// once keeps the maps from rehashing a few hundred entries at a time,
    /// which is most of what a cold plan costs.
    pub fn reserve(&mut self, ground_cells: usize, tree_cells: usize) {
        self.ground.reserve(ground_cells);
        self.trees.reserve(tree_cells);
    }

    /// The cell's population, generating it if it is not memoised. `None` means
    /// the budget was exhausted; the cell may still be memoised by a later call.
    fn ensure_ground(
        &mut self,
        seed: u64,
        cell: [i32; 2],
        remaining: &mut usize,
    ) -> Option<FloraCell> {
        if let Some(cached) = self.ground.get(&cell) {
            return Some(*cached);
        }
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        self.samples += 1;
        let sample = flora_cell(seed, cell[0], cell[1]);
        self.ground.insert(cell, sample);
        Some(sample)
    }

    /// Memoise the tree cell if it is not already known and return its tree.
    /// `None` means the budget was exhausted before the cell could be
    /// generated; `Some(None)` means the cell holds no tree.
    fn tree(
        &mut self,
        seed: u64,
        cell: [i32; 2],
        remaining: &mut usize,
    ) -> Option<Option<FloraSite>> {
        if let Some(cached) = self.trees.get(&cell) {
            return Some(*cached);
        }
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        self.samples += 1;
        let sample = tree_cell(seed, cell[0], cell[1]);
        self.trees.insert(cell, sample);
        Some(sample)
    }

    /// Keep only the cells the completed plan's windows cover. Inclusive
    /// `[min_x, max_x, min_z, max_z]` bounds, in lattice cells; `None` for the
    /// ground window means the tier set carries no ground-cover cells at all.
    fn trim(&mut self, ground: Option<[i32; 4]>, trees: [i32; 4]) {
        self.ground.retain(|cell, _| {
            ground.is_some_and(|window| {
                (window[0]..=window[1]).contains(&cell[0])
                    && (window[2]..=window[3]).contains(&cell[1])
            })
        });
        self.trees.retain(|cell, _| {
            (trees[0]..=trees[1]).contains(&cell[0]) && (trees[2]..=trees[3]).contains(&cell[1])
        });
    }
}

/// [`plan_flora_into`] over a caller-owned cell memo, generating at most
/// `sample_budget` cells that are not yet memoised.
///
/// Returns whether the plan is complete. On `false`, `plan` holds a partial list
/// the caller must not publish, but every cell the call reached is already in
/// `cache`, so a retry restarts cheaply and continues past the point the budget
/// stopped it. `usize::MAX` always completes.
///
/// The completed output is identical to [`plan_flora_into`] for the same
/// arguments. The memo changes only where a cell's population comes from, and
/// ground cells are generated in the same ascending lattice order as the plain
/// planner visits them.
#[allow(clippy::too_many_arguments)] // `plan_flora_into`'s arguments plus the memo and its budget
pub fn plan_flora_cached_into(
    seed: u64,
    eye: [f32; 3],
    tiers: &[FloraTier],
    max_sites: usize,
    max_trees: usize,
    sample_budget: usize,
    cache: &mut FloraCellCache,
    plan: &mut FloraPlan,
) -> bool {
    plan.clear();
    let mut remaining = sample_budget;
    let eye_x = eye_metre(eye[0]);
    let eye_z = eye_metre(eye[2]);
    let outer = tiers.iter().map(|tier| tier.radius_m).max().unwrap_or(0);
    if cache.ground_cells() == 0 && cache.tree_cells() == 0 {
        // A cold cache is about to be filled with a whole window of cells.
        let ground_side = (2 * outer / FLORA_CELL_M + 1).max(0) as usize;
        let tree_side = (2 * LANDSCAPE_TREE_RADIUS_M / TREE_CELL_M + 1).max(0) as usize;
        cache.reserve(ground_side * ground_side, tree_side * tree_side);
    }
    let mut ground_window: Option<[i32; 4]> = None;
    if outer > 0 {
        let first = (eye_x - outer).div_euclid(FLORA_CELL_M);
        let last = (eye_x + outer).div_euclid(FLORA_CELL_M);
        let first_z = (eye_z - outer).div_euclid(FLORA_CELL_M);
        let last_z = (eye_z + outer).div_euclid(FLORA_CELL_M);
        ground_window = Some([first, last, first_z, last_z]);
        for cell_z in first_z..=last_z {
            for cell_x in first..=last {
                let origin_x = cell_x * FLORA_CELL_M;
                let origin_z = cell_z * FLORA_CELL_M;
                let distance = chebyshev(origin_x, origin_z, eye_x, eye_z);
                let Some(tier) = tier_of(tiers, distance) else {
                    continue;
                };
                // Density is continuous in distance and the survival threshold
                // is a hash of the cell, so a band boundary thins the field
                // gradually instead of switching it off at a fence.
                if !keeps_cell(seed, cell_x, cell_z, density_percent_at(tiers, distance)) {
                    continue;
                }
                let Some(cell) = cache.ensure_ground(seed, [cell_x, cell_z], &mut remaining) else {
                    return false;
                };
                for site in cell.sites() {
                    if plan.sites.len() >= max_sites {
                        plan.dropped += 1;
                        continue;
                    }
                    plan.sites.push(PlacedFlora::from_site(*site, tier, seed));
                }
            }
        }
    }
    let last_tier = tiers.len().saturating_sub(1) as u8;
    let first = (eye_x - LANDSCAPE_TREE_RADIUS_M).div_euclid(TREE_CELL_M);
    let last = (eye_x + LANDSCAPE_TREE_RADIUS_M).div_euclid(TREE_CELL_M);
    let first_z = (eye_z - LANDSCAPE_TREE_RADIUS_M).div_euclid(TREE_CELL_M);
    let last_z = (eye_z + LANDSCAPE_TREE_RADIUS_M).div_euclid(TREE_CELL_M);
    for cell_z in first_z..=last_z {
        for cell_x in first..=last {
            let Some(site) = cache.tree(seed, [cell_x, cell_z], &mut remaining) else {
                return false;
            };
            let Some(site) = site else {
                continue;
            };
            if chebyshev(site.x, site.z, eye_x, eye_z) > LANDSCAPE_TREE_RADIUS_M {
                continue;
            }
            if plan.trees.len() >= max_trees {
                plan.dropped += 1;
                continue;
            }
            let tier = tier_of(tiers, chebyshev(site.x, site.z, eye_x, eye_z)).unwrap_or(last_tier);
            plan.trees.push(PlacedFlora::from_site(site, tier, seed));
        }
    }
    cache.trim(ground_window, [first, last, first_z, last_z]);
    true
}

/// Incremental flora placement field for one seed and tier configuration.
///
/// The field owns a [`FloraCellCache`] and the last committed [`FloraPlan`].
/// [`Self::advance`] moves the field to an eye position by re-running
/// [`plan_flora_cached_into`], so only the cells the new window adds are
/// generated and the committed plan is exactly what [`plan_flora`] returns for
/// the same eye. Work is admitted in bounded slices: an `advance` that runs out
/// of budget returns `false` with the previous plan untouched, and the next
/// call continues from the memo.
///
/// The tier slice and the caps are fixed for the field's lifetime: pass the
/// same values on every call.
#[derive(Clone, Debug)]
pub struct FloraField {
    seed: u64,
    cache: FloraCellCache,
    plan: FloraPlan,
    pending: FloraPlan,
    anchor: Option<[i32; 2]>,
    generation: u64,
}

impl FloraField {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            cache: FloraCellCache::default(),
            plan: FloraPlan::default(),
            pending: FloraPlan::default(),
            anchor: None,
            generation: 0,
        }
    }

    /// Eye metre position of the committed plan, `None` before the first
    /// commit. The x and z axes are returned in that order.
    pub fn anchor(&self) -> Option<[i32; 2]> {
        self.anchor
    }

    /// Number of committed plans. Increments once per completed advance, so a
    /// caller can tell a rebuild from re-uploading the same plan.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The committed plan. Equal to [`plan_flora`] for the committed eye.
    pub fn plan(&self) -> &FloraPlan {
        &self.plan
    }

    /// Cells the field has generated over its lifetime.
    pub fn samples(&self) -> u64 {
        self.cache.samples()
    }

    /// Ground-cover cells currently memoised.
    pub fn cached_ground_cells(&self) -> usize {
        self.cache.ground_cells()
    }

    /// Tree cells currently memoised.
    pub fn cached_tree_cells(&self) -> usize {
        self.cache.tree_cells()
    }

    /// Advance the field to `eye`, generating at most `sample_budget` cells.
    ///
    /// Returns `true` when [`Self::plan`] is the plan for `eye`; repeating the
    /// call for the same eye is then free. Returns `false` when the budget ran
    /// out: the previous committed plan is left in place and the next call
    /// carries on. A non-finite eye folds to zero exactly as [`plan_flora`]
    /// does.
    pub fn advance(
        &mut self,
        eye: [f32; 3],
        tiers: &[FloraTier],
        max_sites: usize,
        max_trees: usize,
        sample_budget: usize,
    ) -> bool {
        let eye_x = eye_metre(eye[0]);
        let eye_z = eye_metre(eye[2]);
        if self.anchor == Some([eye_x, eye_z]) {
            return true;
        }
        let complete = plan_flora_cached_into(
            self.seed,
            eye,
            tiers,
            max_sites,
            max_trees,
            sample_budget,
            &mut self.cache,
            &mut self.pending,
        );
        if !complete {
            return false;
        }
        // Publish atomically, and hand the old plan's buffers to the next
        // in-flight attempt instead of reallocating them.
        std::mem::swap(&mut self.plan, &mut self.pending);
        self.anchor = Some([eye_x, eye_z]);
        self.generation = self.generation.wrapping_add(1);
        true
    }
}

// -- Distance levels ---------------------------------------------------------

/// Coarse cell edge in metres for a level: level 0 is one metre.
pub fn lod_cell_m(level: u32) -> i32 {
    1i32 << level.min(MAX_LOD_LEVEL)
}

/// Surface sample at one coarse grid vertex, at its exact metre coordinate.
///
/// Exact sampling (rather than a cell maximum) keeps the sample a pure function
/// of the coordinate, so [`lod_sample`] and the tile mesher can never disagree
/// about what the generator says at a point. Relief narrower than the level's
/// cell is not represented at that level.
pub fn lod_vertex(seed: u64, x: i32, z: i32) -> LodSample {
    let column = column(seed, x, z);
    if column.flooded() {
        LodSample::default()
    } else {
        LodSample {
            height: column.height,
            surface: column.surface,
            biome: column.biome,
            flooded: false,
        }
    }
}

/// Conservative aggregate over one covered coarse cell: the maximum terrain
/// height and whether every sampled column is flooded. Sampling is a bounded
/// lattice, so a peak narrower than the lattice can be missed; that is the
/// stated limitation of a coarse level and what silhouette checks measure.
pub fn lod_sample(seed: u64, level: u32, cell_x: i32, cell_z: i32) -> LodSample {
    let cell_m = lod_cell_m(level);
    // Bounded to a 16x16 lattice per cell, so a coarse cell never costs more than a
    // fine one even at the horizon levels.
    let step = (cell_m / 16).max(1);
    let origin_x = cell_x * cell_m;
    let origin_z = cell_z * cell_m;
    let mut best: Option<Column> = None;
    let mut flooded = true;
    let mut z = origin_z;
    while z < origin_z + cell_m {
        let mut x = origin_x;
        while x < origin_x + cell_m {
            let column = column(seed, x, z);
            flooded &= column.flooded();
            if best.is_none_or(|top| column.height > top.height) {
                best = Some(column);
            }
            x += step;
        }
        z += step;
    }
    let best = best.expect("a coarse cell always covers at least one column");
    if flooded {
        LodSample::default()
    } else {
        LodSample {
            height: best.height,
            surface: best.surface,
            biome: best.biome,
            flooded: false,
        }
    }
}

/// Build the derived surface tile for one `LOD_TILE_CELLS` block of coarse cells.
///
/// Every drawn cell is one flat quad at its own [`lod_step_m`]-quantised height,
/// and every difference to a neighbouring cell is a vertical wall spanning the
/// full step: the surface reads as stacked voxel blocks instead of as a smoothed
/// heightfield. Top and wall vertices carry the face's own normal and colour, so
/// the light/dark break between a top and the wall under it is a real shading
/// break rather than a gradient.
///
/// Coplanar cells are greedy-merged into as few quads as possible. A face
/// between two drawn cells belongs to the higher of them, which is what keeps a
/// tile border crack-free without doubling the wall: two tiles sharing a border
/// sample the same two cell heights across it and reach the same decision, so
/// exactly one of them emits the face.
///
/// Cells whose centre lies in `filter.hole`, or outside `filter.bound`, are
/// omitted, and every face against an omitted cell hangs at least
/// [`FILTER_WALL_CELLS`] cells below its own top. Those walls replace the skirts
/// an earlier version added along the same boundaries: a finer ring in a hole,
/// or a coarser ring beyond a bound, cannot show daylight through the seam.
pub fn lod_tile_mesh(seed: u64, level: u32, key: [i32; 2], filter: TileFilter) -> Mesh {
    let level = level.min(MAX_LOD_LEVEL);
    let cell_m = lod_cell_m(level);
    let step = lod_step_m(level);
    debug_assert!(
        (key[0] as i64 * LOD_TILE_CELLS as i64 + LOD_TILE_CELLS as i64) * cell_m as i64
            <= i32::MAX as i64
            && (key[0] as i64 * LOD_TILE_CELLS as i64 + LOD_TILE_CELLS as i64) * cell_m as i64
                >= i32::MIN as i64,
        "tile key {key:?} leaves the representable metre range at level {level}"
    );
    // One terrain sample per coarse cell centre, cached so a wall and the two
    // cells it separates all read the same number. The grid carries a one-cell
    // halo: a wall on a tile border is decided from the neighbour's own cell
    // centre across it, which is the same sample the neighbouring tile takes.
    let mut surface = Surface::default();
    for cz in -1..=LOD_TILE_CELLS {
        for cx in -1..=LOD_TILE_CELLS {
            let drawn = cell_drawn(&filter, key, cell_m, cx, cz);
            surface.drawn[(cz + 1) as usize][(cx + 1) as usize] = drawn;
            // Undrawn cells are sampled too: a wall on a hole or bound edge
            // needs the ground across it to know how far down to reach.
            let x = (key[0] * LOD_TILE_CELLS + cx) * cell_m + cell_m / 2;
            let z = (key[1] * LOD_TILE_CELLS + cz) * cell_m + cell_m / 2;
            // The column is sampled directly rather than through
            // [`lod_vertex`] so the wall's sub-surface material comes from the
            // same sample as the top, at no extra cost.
            let sample = column(seed, x, z);
            // A flooded cell normally draws the sea surface. Inside the
            // filter's bed square a finer water surface covers the cells, so
            // this level draws the ground instead - see [`TileFilter::bed`].
            let as_bed = sample.flooded() && bed_contains(&filter, x, z);
            let (height, top_material) = if sample.flooded() && !as_bed {
                (SEA_LEVEL, material::WATER)
            } else {
                (sample.height, sample.surface)
            };
            surface.sample[(cz + 1) as usize][(cx + 1) as usize] = CellSample {
                // Floor to the level's step: a cell top is never raised above
                // the ground it stands for, so a ring never occludes the finer
                // mesh inside it, and a cell that is entirely above sea level
                // and below one step still merges with the water it borders.
                height: quantise_height(height, step),
                colour: material::color(top_material),
                // The cell's world metre position: centre in x and z, its drawn
                // top in y. Two tiles sampling the same cell agree exactly.
                tone: material::tone(top_material, x, height, z),
                sub_colour: material::color(sample.sub_surface),
                sub_tone: material::tone(sample.sub_surface, x, height, z),
                rock_tone: material::tone(material::STONE, x, height, z),
                sub_depth: sample.sub_depth,
            };
        }
    }
    if !any_cell(&surface) {
        // No cells: an empty mesh is the correct representation, not a failure.
        return Mesh::default();
    }

    let mut mesh = Mesh::default();
    let mut tops = Vec::new();
    merge_tops(&surface, &mut tops);
    for rect in tops {
        emit_top(&mut mesh, rect, key, cell_m);
    }
    let mut walls = Vec::new();
    merge_walls(&surface, cell_m, &mut walls);
    for run in walls {
        emit_wall(&mut mesh, run, key, cell_m);
    }
    mesh
}

/// Extra halo cells either side of the tile's own grid.
const TILE_HALO: i32 = 1;
/// Cell index extent of one tile's sampled grid, halo included.
const TILE_GRID: usize = (LOD_TILE_CELLS + 2 * TILE_HALO) as usize;

/// Cells a wall reaches below its own top when the cell across the face is not
/// drawn at this level: the depth the previous version's boundary skirts used.
const FILTER_WALL_CELLS: i32 = 2;

/// The quantised surface of one coarse cell.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CellSample {
    /// Top height in metres; the whole cell is flat at this height.
    height: i32,
    /// Top-face base colour, before the per-cell tone.
    colour: [f32; 3],
    /// Deterministic tone of the top material at this cell, in 1/256ths.
    tone: [i32; 3],
    /// Base colour of the sub-surface layer, which is what a wall shows.
    sub_colour: [f32; 3],
    /// Tone of that sub-surface layer at this cell.
    sub_tone: [i32; 3],
    /// Tone of bare rock at this cell, for a wall reaching past the sub layer.
    rock_tone: [i32; 3],
    /// Depth of that sub-surface layer in metres.
    sub_depth: i32,
}

impl CellSample {
    /// Base colour and tone of a wall hanging from this cell's top down to
    /// `foot`.
    ///
    /// A step that stays inside the sub-surface layer shows that layer - soil
    /// under grass, stone under snow - and anything deeper is bare rock. This is
    /// the same material [`material_in_column`] reports for the voxels the wall
    /// stands for, and it is what makes a wall read as a break rather than as a
    /// darker copy of the top above it. The tone comes from the cell's own
    /// position, so the two layers of one wall do not share a tone.
    fn wall_colour(&self, foot: i32) -> ([f32; 3], [i32; 3]) {
        if self.height - foot <= self.sub_depth {
            (self.sub_colour, self.sub_tone)
        } else {
            (material::color(material::STONE), self.rock_tone)
        }
    }
}

/// One tile's sampled cells: heights and colours at cell centres over the tile
/// plus [`TILE_HALO`], and which of those cells this level draws.
struct Surface {
    sample: [[CellSample; TILE_GRID]; TILE_GRID],
    drawn: [[bool; TILE_GRID]; TILE_GRID],
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            sample: [[CellSample::default(); TILE_GRID]; TILE_GRID],
            drawn: [[false; TILE_GRID]; TILE_GRID],
        }
    }
}

impl Surface {
    /// The cell at halo-relative index `(cx, cz)`, which must lie in
    /// `-TILE_HALO..=LOD_TILE_CELLS + TILE_HALO`.
    fn sample_at(&self, cx: i32, cz: i32) -> CellSample {
        self.sample[(cz + TILE_HALO) as usize][(cx + TILE_HALO) as usize]
    }

    fn drawn(&self, cx: i32, cz: i32) -> bool {
        self.drawn[(cz + TILE_HALO) as usize][(cx + TILE_HALO) as usize]
    }
}

/// Whether this level draws the cell at halo-relative index `(cx, cz)`.
///
/// Inside the tile the answer is the filter's: a cell whose centre lies in the
/// hole, or outside the bound, is not drawn. Outside the tile the answer is
/// always yes, and deliberately not the filter's. A halo cell belongs to a
/// neighbouring tile of the same ring, and the sample normalises a tile's clip
/// to the tile before caching a mesh (`normalized_filter`), so letting the
/// filter decide a halo cell would make two filters that produce the same
/// geometry produce different meshes. The neighbouring tile draws the cell
/// whenever this ring's square covers it, which is exactly when the face between
/// the two cells is a real step at this level.
fn cell_drawn(filter: &TileFilter, key: [i32; 2], cell_m: i32, cx: i32, cz: i32) -> bool {
    if cx < 0 || cz < 0 || cx >= LOD_TILE_CELLS || cz >= LOD_TILE_CELLS {
        return true;
    }
    let centre = [
        (key[0] * LOD_TILE_CELLS + cx) * cell_m + cell_m / 2,
        (key[1] * LOD_TILE_CELLS + cz) * cell_m + cell_m / 2,
    ];
    let in_hole = filter.hole.is_some_and(|hole| hole.contains_centre(centre));
    let in_bound = filter
        .bound
        .is_none_or(|bound| bound.contains_centre(centre));
    !in_hole && in_bound
}

/// Whether the bed square of `filter` covers the cell at metre point `(x, z)`.
/// Measured on the plant position, not the tile's local grid, because a bed
/// square is not tile-aligned the way a ring's own bound is.
fn bed_contains(filter: &TileFilter, x: i32, z: i32) -> bool {
    filter.bed.is_some_and(|bed| bed.contains_centre([x, z]))
}

fn any_cell(surface: &Surface) -> bool {
    (0..LOD_TILE_CELLS).any(|cz| (0..LOD_TILE_CELLS).any(|cx| surface.drawn(cx, cz)))
}

/// Floor a height to a multiple of `step`. Integer division rounds toward
/// negative infinity, so this is exact for the negative heights a column can
/// report below sea level as well.
fn quantise_height(height: i32, step: i32) -> i32 {
    let step = step.max(1);
    height.div_euclid(step) * step
}

/// One greedy-merged top rectangle, in cell indices inside the tile.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TopRect {
    origin: [i32; 2],
    extent: [i32; 2],
    height: i32,
    colour: [f32; 3],
    /// Mean tone of the rectangle's cells, applied when the quad is emitted.
    tone: [i32; 3],
}

/// Greedy-merge the drawn cells into as few flat rectangles as possible.
///
/// Scan order is row-major: the rectangle grows as far as it can along +x and
/// then along +z while every cell in the row stays equal in height and top
/// colour. Cells that differ in either, and cells the filter cut out, end the
/// run; what a cell's walls look like does not, which is the point of merging
/// tops by their own two fields. A flat region therefore costs one quad however
/// many cells it holds, and equal input always produces the same rectangles.
fn merge_tops(surface: &Surface, rects: &mut Vec<TopRect>) {
    let mut taken = [[false; LOD_TILE_CELLS as usize]; LOD_TILE_CELLS as usize];
    let free = |taken: &[[bool; LOD_TILE_CELLS as usize]],
                cx: i32,
                cz: i32,
                top: i32,
                colour: [f32; 3]| {
        let slot = (cz as usize, cx as usize);
        if !surface.drawn(cx, cz) || taken[slot.0][slot.1] {
            return false;
        }
        let cell = surface.sample_at(cx, cz);
        cell.height == top && cell.colour == colour
    };
    for cz in 0..LOD_TILE_CELLS {
        for cx in 0..LOD_TILE_CELLS {
            if !surface.drawn(cx, cz) || taken[cz as usize][cx as usize] {
                continue;
            }
            let cell = surface.sample_at(cx, cz);
            let mut width = 1;
            while cx + width < LOD_TILE_CELLS
                && free(&taken, cx + width, cz, cell.height, cell.colour)
            {
                width += 1;
            }
            let mut depth = 1;
            'grow: while cz + depth < LOD_TILE_CELLS {
                for step in 0..width {
                    if !free(&taken, cx + step, cz + depth, cell.height, cell.colour) {
                        break 'grow;
                    }
                }
                depth += 1;
            }
            for z in cz..cz + depth {
                for x in cx..cx + width {
                    taken[z as usize][x as usize] = true;
                }
            }
            rects.push(TopRect {
                origin: [cx, cz],
                extent: [width, depth],
                height: cell.height,
                colour: cell.colour,
                tone: merged_tone(surface, cx, cz, width, depth),
            });
        }
    }
}

/// Mean top tone over a rectangle of cells: the aggregate a merged top carries
/// instead of the tone of one sampled cell.
fn merged_tone(surface: &Surface, cx: i32, cz: i32, width: i32, depth: i32) -> [i32; 3] {
    let mut sum = [0i32; 3];
    for z in cz..cz + depth {
        for x in cx..cx + width {
            let tone = surface.sample_at(x, z).tone;
            for channel in 0..3 {
                sum[channel] += tone[channel];
            }
        }
    }
    material::mean_tone(sum, width * depth)
}

/// One greedy-merged wall: a run of cell faces in one plane sharing top, foot
/// and base colour.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WallRun {
    /// Outward normal of the wall in cell axes: one of `-x`, `+x`, `-z`, `+z`.
    normal: [i32; 3],
    /// Cell index of the plane the wall lies in, along the normal's axis.
    plane: i32,
    /// First cell index and cell count along the face's own axis: z for an
    /// x-normal wall, x for a z-normal wall.
    start: i32,
    len: i32,
    /// Wall top and foot in metres, `top > foot`.
    top: i32,
    foot: i32,
    /// Base wall colour, before the run's aggregate tone.
    colour: [f32; 3],
    /// Mean tone of the run's cells, in 1/256ths.
    tone: [i32; 3],
}

/// The wall one cell face needs: top, foot, base colour and the cell's own
/// tone, or `None` for a face that needs no wall.
///
/// Between two drawn cells of this level the higher cell owns the face, so
/// exactly one of the two tiles that can see the pair emits it and the wall
/// spans the step exactly. Against a cell this level does not draw - inside a
/// hole, beyond a bound - there is no same-level neighbour to meet, so the wall
/// hangs at least [`FILTER_WALL_CELLS`] cells below its own top, and reaches
/// further down when the neighbour across the edge stands lower than that.
///
/// The base colour, not the tone, decides whether two faces merge: a run is
/// tinted once with the mean tone of its cells, so colour variation never
/// shortens a run that shares a plane and a layer.
fn wall_spec(
    surface: &Surface,
    cell_m: i32,
    cx: i32,
    cz: i32,
    offset: [i32; 2],
) -> Option<(i32, i32, [f32; 3], [i32; 3])> {
    if !surface.drawn(cx, cz) {
        return None;
    }
    let own = surface.sample_at(cx, cz);
    let (nx, nz) = (cx + offset[0], cz + offset[1]);
    let neighbour = surface.sample_at(nx, nz);
    let (foot, top) = if surface.drawn(nx, nz) {
        if own.height <= neighbour.height {
            return None;
        }
        (neighbour.height, own.height)
    } else {
        let fallback = own.height - FILTER_WALL_CELLS * cell_m;
        (neighbour.height.min(fallback), own.height)
    };
    let (colour, tone) = own.wall_colour(foot);
    Some((top, foot, colour, tone))
}

/// Greedy-merge the walls of every drawn cell into runs.
///
/// An x-normal wall runs along z and a z-normal wall along x, so each family is
/// a grid of (plane, run) faces that merge while top, foot and colour all stay
/// equal. The plane is the cell index the face sits on: the cell's own index for
/// a `-x`/`-z` face, and the next one for a `+x`/`+z` face.
fn merge_walls(surface: &Surface, cell_m: i32, runs: &mut Vec<WallRun>) {
    for (normal, offset) in [
        ([-1, 0, 0], [-1, 0]),
        ([1, 0, 0], [1, 0]),
        ([0, 0, -1], [0, -1]),
        ([0, 0, 1], [0, 1]),
    ] {
        // Faces of this family run along x when their normal is on z.
        let along_x = normal[2] != 0;
        let plane_step = if along_x {
            normal[2].max(0)
        } else {
            normal[0].max(0)
        };
        for plane in 0..LOD_TILE_CELLS {
            let at = |run: i32| {
                if along_x {
                    (run, plane)
                } else {
                    (plane, run)
                }
            };
            let mut run = 0;
            while run < LOD_TILE_CELLS {
                let (cx, cz) = at(run);
                let Some(spec) = wall_spec(surface, cell_m, cx, cz, offset) else {
                    run += 1;
                    continue;
                };
                let mut len = 1;
                let mut tone_sum = spec.3;
                while run + len < LOD_TILE_CELLS {
                    let (nx, nz) = at(run + len);
                    let Some(next) = wall_spec(surface, cell_m, nx, nz, offset) else {
                        break;
                    };
                    // Base colour, top and foot are the merge key; the tone is
                    // aggregated below, so variation cannot shorten a run.
                    if (next.0, next.1, next.2) != (spec.0, spec.1, spec.2) {
                        break;
                    }
                    for (channel, sum) in tone_sum.iter_mut().enumerate() {
                        *sum += next.3[channel];
                    }
                    len += 1;
                }
                runs.push(WallRun {
                    normal,
                    plane: plane + plane_step,
                    start: run,
                    len,
                    top: spec.0,
                    foot: spec.1,
                    colour: spec.2,
                    tone: material::mean_tone(tone_sum, len),
                });
                run += len;
            }
        }
    }
}

/// Push one merged top rectangle as two flat-shaded triangles. Winding follows
/// the core mesher: `cross(U, V)` with `U = +x`, `V = -z` points at `+Y`.
fn emit_top(mesh: &mut Mesh, rect: TopRect, key: [i32; 2], cell_m: i32) {
    let x0 = (key[0] * LOD_TILE_CELLS + rect.origin[0]) * cell_m;
    let z0 = (key[1] * LOD_TILE_CELLS + rect.origin[1]) * cell_m;
    let x1 = x0 + rect.extent[0] * cell_m;
    let z1 = z0 + rect.extent[1] * cell_m;
    let base = mesh.vertices.len() as u32;
    for position in [
        [x0, rect.height, z1],
        [x1, rect.height, z1],
        [x1, rect.height, z0],
        [x0, rect.height, z0],
    ] {
        mesh.vertices.push(Vertex {
            position: position.map(|v| v as f32),
            normal: [0.0, 1.0, 0.0],
            color: material::tint(rect.colour, rect.tone),
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Push one merged wall as two flat-shaded triangles. Corner order is chosen so
/// `cross(p1 - p0, p2 - p0)` points along the wall's outward normal, which is
/// what the main pass's back-face culling reads.
fn emit_wall(mesh: &mut Mesh, run: WallRun, key: [i32; 2], cell_m: i32) {
    let x_at = |cell: i32| ((key[0] * LOD_TILE_CELLS + cell) * cell_m) as f32;
    let z_at = |cell: i32| ((key[1] * LOD_TILE_CELLS + cell) * cell_m) as f32;
    let low = run.foot as f32;
    let high = run.top as f32;
    let corners = match run.normal {
        // West: outward -x, running +z.
        [-1, 0, 0] => [
            [x_at(run.plane), low, z_at(run.start)],
            [x_at(run.plane), low, z_at(run.start + run.len)],
            [x_at(run.plane), high, z_at(run.start + run.len)],
            [x_at(run.plane), high, z_at(run.start)],
        ],
        // East: outward +x, running -z.
        [1, 0, 0] => [
            [x_at(run.plane), low, z_at(run.start + run.len)],
            [x_at(run.plane), low, z_at(run.start)],
            [x_at(run.plane), high, z_at(run.start)],
            [x_at(run.plane), high, z_at(run.start + run.len)],
        ],
        // North: outward -z, running -x.
        [0, 0, -1] => [
            [x_at(run.start + run.len), low, z_at(run.plane)],
            [x_at(run.start), low, z_at(run.plane)],
            [x_at(run.start), high, z_at(run.plane)],
            [x_at(run.start + run.len), high, z_at(run.plane)],
        ],
        // South: outward +z, running +x.
        _ => [
            [x_at(run.start), low, z_at(run.plane)],
            [x_at(run.start + run.len), low, z_at(run.plane)],
            [x_at(run.start + run.len), high, z_at(run.plane)],
            [x_at(run.start), high, z_at(run.plane)],
        ],
    };
    let base = mesh.vertices.len() as u32;
    for position in corners {
        mesh.vertices.push(Vertex {
            position,
            normal: run.normal.map(|v| v as f32),
            color: material::tint(run.colour, run.tone),
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

// -- Distance rings ----------------------------------------------------------

/// One nested distance ring: a coarse level and the half-extent in metres of the
/// square it covers around the eye.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingConfig {
    pub level: u32,
    pub half_extent: i32,
}

impl RingConfig {
    /// Metre edge of one tile at this ring's level.
    pub fn tile_size_m(&self) -> i32 {
        LOD_TILE_CELLS * lod_cell_m(self.level)
    }
}

/// The shipped ring set: a geometric cascade that doubles the cell size at every
/// step, 2 m cells from the streaming window out to 256 m, then 4, 8, 16, 32 and
/// 64 m cells out to 8192 m. Doubling keeps the near field fine (a 4x jump from a
/// 1 m window straight to 4 m cells makes the first ring read as stair-steps under
/// the camera) and holds the triangle count of every ring roughly constant.
///
/// Each half-extent is a whole number of that ring's tiles, and each ring's edges
/// are a multiple of the next ring's cell size, so a ring boundary always falls on
/// a cell boundary of the ring that cuts it out.
pub const LANDSCAPE_RINGS: [RingConfig; 6] = [
    RingConfig {
        level: 1,
        half_extent: 256,
    },
    RingConfig {
        level: 2,
        half_extent: 512,
    },
    RingConfig {
        level: 3,
        half_extent: 1024,
    },
    RingConfig {
        level: 4,
        half_extent: 2048,
    },
    RingConfig {
        level: 5,
        half_extent: 4096,
    },
    RingConfig {
        level: 6,
        half_extent: 8192,
    },
];

/// Height quantisation step in metres for a level: the smallest step that still
/// reads as a step at the distance that level covers.
///
/// The derivation, per ring, from [`LANDSCAPE_RINGS`]: a step `s` at distance `d`
/// subtends `s/d`; the landscape projection is 65 degrees over a 1440 pixel
/// tall viewport (the reference device in landscape), so one pixel is
/// `1.134/1440 = 7.9e-4` rad and a step is visible at `s >= 2*7.9e-4*d`. Taking
/// the ring's *outer* half-extent as the distance, because that is where its
/// steps are smallest on screen, and rounding to the nearest power of two (never
/// below the world's 1 m voxel, which is as fine as a step can be):
///
/// | level | cell | outer half-extent | 2 px needs | step |
/// | --- | --- | --- | --- | --- |
/// | 1 | 2 m | 256 m | 0.40 m | 1 m |
/// | 2 | 4 m | 512 m | 0.81 m | 1 m |
/// | 3 | 8 m | 1024 m | 1.6 m | 2 m |
/// | 4 | 16 m | 2048 m | 3.2 m | 4 m |
/// | 5 | 32 m | 4096 m | 6.5 m | 8 m |
/// | 6 | 64 m | 8192 m | 12.9 m | 16 m |
///
/// Level 0 (1 m cells) is not used by a ring and is exact at 1 m. Fog, not the
/// step, is what limits contrast in the outer two rings - aerial perspective
/// leaves about 9% of a surface's own colour at 4 km - so the step there is
/// sized to keep the silhouette blocky rather than to make every face read.
pub const LOD_STEP_M: [i32; MAX_LOD_LEVEL as usize + 1] = [1, 1, 1, 2, 4, 8, 16];

/// Height quantisation step of a distance level, in metres. See [`LOD_STEP_M`].
pub fn lod_step_m(level: u32) -> i32 {
    LOD_STEP_M[level.min(MAX_LOD_LEVEL) as usize]
}

/// One tile the renderer should have resident this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingTile {
    pub level: u32,
    pub key: [i32; 2],
    pub filter: TileFilter,
}

impl RingTile {
    /// Whether this tile draws the coarse cell of its own level that contains
    /// the world metre point `(x, z)`.
    ///
    /// The tile owns the cells of `LOD_TILE_CELLS` square starting at its key,
    /// and draws the ones its filter keeps; a caller uses this to ask whether
    /// geometry built for this tile covers the ground under a camera.
    pub fn covers_point(&self, x: i32, z: i32) -> bool {
        let cell_m = lod_cell_m(self.level);
        let cell = [x.div_euclid(cell_m), z.div_euclid(cell_m)];
        cell[0].div_euclid(LOD_TILE_CELLS) == self.key[0]
            && cell[1].div_euclid(LOD_TILE_CELLS) == self.key[1]
            && self.filter.covers_point(self.level, x, z)
    }
}

/// Largest world metre a `f32` eye coordinate is folded into. Far beyond the
/// simulation domain, but finite, so a runaway camera cannot overflow the plan.
const EYE_LIMIT: f32 = 1.0e9;

fn eye_metre(value: f32) -> i32 {
    if value.is_finite() {
        value.clamp(-EYE_LIMIT, EYE_LIMIT).floor() as i32
    } else {
        0
    }
}

/// Largest multiple of `size` not greater than `value`.
fn snap(value: i32, size: i32) -> i32 {
    let size = size.max(1);
    size * value.div_euclid(size)
}

/// The authoritative streaming window around `eye`, in world metres.
///
/// This is the [`STREAM_RADIUS_CHUNKS`] square of chunks centred on the eye's
/// chunk, clamped to the world square, so it is exactly the square
/// [`crate::World::stream_resident_chunks`] publishes for any eye inside the
/// world. Its edges are chunk-aligned, hence aligned to every ring's cell size.
/// It names where the authoritative one-metre meshes are drawn - and where the
/// innermost ring draws its own coarser ground *under* them, because residency
/// lags the eye and a hole here is a hole under the player. See [`ring_plan`].
pub fn fine_clip(eye: [f32; 3]) -> Clip {
    // The ring planner serves the landscape source, whose domain is much wider
    // than the legacy sandbox: a camera anywhere in it still gets a full window.
    let limit = crate::LANDSCAPE_WORLD_LIMIT / CHUNK_EDGE;
    let centre =
        [eye[0], eye[2]].map(|v| eye_metre(v).div_euclid(CHUNK_EDGE).clamp(-limit, limit - 1));
    let low = centre.map(|c| (c - STREAM_RADIUS_CHUNKS).max(-limit) * CHUNK_EDGE);
    let high = centre.map(|c| ((c + STREAM_RADIUS_CHUNKS).min(limit - 1) + 1) * CHUNK_EDGE);
    Clip {
        min: low,
        max: high,
    }
}

/// Plan the resident far-terrain tiles for one eye position.
///
/// Ring `i` covers its bound square minus ring `i-1`'s bound square. The
/// innermost ring is not cut at all: it draws its whole square, and inside the
/// `fine` square - the authoritative streaming window [`fine_clip`] names - its
/// flooded cells as the ground under them rather than as the sea surface. Every
/// ground cell inside the outermost square is therefore drawn by exactly one
/// ring tile, at every eye position, whether or not the authoritative chunk
/// meshes have arrived yet. Inside the window the chunk meshes are drawn over
/// that ground (they stand a metre above the ring's quantised surface), and the
/// fine water mesh is drawn over the bed; before either arrives the ring's own
/// ground is what the player stands on and what the sea floor is.
///
/// An earlier version cut the innermost ring with the streaming window, which
/// made the window a promise that fine geometry exists. It does not exist until
/// the meshes are generated and uploaded, so a fresh start, a window that just
/// moved or a renderer recreated after a suspend showed clear colour under the
/// camera. Taking the hole only when the fine geometry is resident would mean
/// rebuilding those tiles whenever residency changes; drawing under it costs one
/// coarse layer, most of which is depth-rejected.
///
/// Every ring's square is centred on the eye snapped to that ring's tile size,
/// so the tile set is stable while the camera moves inside one tile, and the
/// same eye and configuration always produce the same ordered list; tiles are
/// emitted nearest ring first and, within a ring, nearest tile first, which is
/// the order a frame's bounded uploads should follow.
///
/// The ground under the window is the *generator's*, one quantised step below
/// the voxel surface. A world whose fine meshes differ from the generator - an
/// edited one - must not use this: removing the top voxel of a column would
/// expose the ring's ground at exactly the height the dig removed. The sample
/// this planner serves makes no edits; one that does needs the ring's hole back,
/// or the underlay lowered by the depth it can be edited to.
///
/// `rings` must be ordered from finest to coarsest with nested squares;
/// [`LANDSCAPE_RINGS`] is that set.
pub fn ring_plan(eye: [f32; 3], fine: Clip, rings: &[RingConfig]) -> Vec<RingTile> {
    let mut plan = Vec::new();
    ring_plan_into(eye, fine, rings, &mut plan);
    plan
}

/// [`ring_plan`] writing into a caller-owned buffer. The buffer is cleared, not
/// reallocated, so a frame loop that keeps one plan buffer allocates nothing
/// after the first frame.
pub fn ring_plan_into(eye: [f32; 3], fine: Clip, rings: &[RingConfig], plan: &mut Vec<RingTile>) {
    plan.clear();
    let centre_x = eye_metre(eye[0]);
    let centre_z = eye_metre(eye[2]);
    let mut hole: Option<Clip> = None;
    let mut bed: Option<Clip> = Some(fine);
    for config in rings {
        let level = config.level.min(MAX_LOD_LEVEL);
        let tile_size = LOD_TILE_CELLS * lod_cell_m(level);
        let half_extent = config.half_extent.max(tile_size);
        let centre = [snap(centre_x, tile_size), snap(centre_z, tile_size)];
        let bound = Clip {
            min: [centre[0] - half_extent, centre[1] - half_extent],
            max: [centre[0] + half_extent, centre[1] + half_extent],
        };
        debug_assert!(
            hole.is_none_or(|hole| bound.min[0] <= hole.min[0]
                && bound.min[1] <= hole.min[1]
                && bound.max[0] >= hole.max[0]
                && bound.max[1] >= hole.max[1]),
            "ring squares must nest: {bound:?} does not contain {hole:?}"
        );
        let filter = TileFilter {
            hole,
            bound: Some(bound),
            bed,
        };
        let first = [
            bound.min[0].div_euclid(tile_size),
            bound.min[1].div_euclid(tile_size),
        ];
        let last = [
            (bound.max[0] - 1).div_euclid(tile_size),
            (bound.max[1] - 1).div_euclid(tile_size),
        ];
        let ring_start = plan.len();
        for key_z in first[1]..=last[1] {
            for key_x in first[0]..=last[0] {
                plan.push(RingTile {
                    level,
                    key: [key_x, key_z],
                    filter,
                });
            }
        }
        // Nearest tile first, so a bounded upload budget fills the ring around
        // the eye before the far side of the square. Ties break on the key, so
        // the order is a pure function of the eye and the ring set. Sorting the
        // plan's own tail keeps the no-allocation property the buffer promises.
        let centre_tile = [
            centre[0].div_euclid(tile_size),
            centre[1].div_euclid(tile_size),
        ];
        plan[ring_start..].sort_unstable_by_key(|tile| {
            let dx = (tile.key[0] - centre_tile[0]).abs();
            let dz = (tile.key[1] - centre_tile[1]).abs();
            (dx.max(dz), dx, dz, tile.key[0], tile.key[1])
        });
        hole = Some(bound);
        bed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tile-sized surface whose every cell, halo included, sits at `height`:
    /// an interior tile of a ring over flat ground. The sub-surface layer is the
    /// same colour and deep enough that every wall shows it.
    fn flat_surface(height: i32) -> Surface {
        let mut surface = Surface::default();
        for cz in -TILE_HALO..=LOD_TILE_CELLS {
            for cx in -TILE_HALO..=LOD_TILE_CELLS {
                let slot = ((cz + TILE_HALO) as usize, (cx + TILE_HALO) as usize);
                surface.drawn[slot.0][slot.1] = true;
                surface.sample[slot.0][slot.1] = CellSample {
                    height,
                    colour: [0.1, 0.2, 0.3],
                    tone: [0; 3],
                    sub_colour: [0.1, 0.2, 0.3],
                    sub_tone: [0; 3],
                    rock_tone: [0; 3],
                    sub_depth: 1_000,
                };
            }
        }
        surface
    }

    fn set(surface: &mut Surface, cx: i32, cz: i32, cell: CellSample) {
        let slot = ((cz + TILE_HALO) as usize, (cx + TILE_HALO) as usize);
        surface.sample[slot.0][slot.1] = cell;
        surface.drawn[slot.0][slot.1] = true;
    }

    fn cell(height: i32, colour: [f32; 3]) -> CellSample {
        CellSample {
            height,
            colour,
            tone: [0; 3],
            sub_colour: colour,
            sub_tone: [0; 3],
            rock_tone: [0; 3],
            sub_depth: 1_000,
        }
    }

    /// Every drawn cell must be covered exactly once, by a rectangle that agrees
    /// with it in height and colour.
    fn assert_tops_cover(surface: &Surface, rects: &[TopRect]) {
        let mut cover: Vec<Vec<Option<TopRect>>> =
            vec![vec![None; LOD_TILE_CELLS as usize]; LOD_TILE_CELLS as usize];
        for rect in rects {
            for z in rect.origin[1]..rect.origin[1] + rect.extent[1] {
                for x in rect.origin[0]..rect.origin[0] + rect.extent[0] {
                    assert!(surface.drawn(x, z), "rectangle covers an undrawn cell");
                    assert_eq!(
                        surface.sample_at(x, z),
                        cell(rect.height, rect.colour),
                        "rectangle disagrees with cell ({x}, {z})"
                    );
                    assert!(
                        cover[z as usize][x as usize].is_none(),
                        "cell ({x}, {z}) covered twice"
                    );
                    cover[z as usize][x as usize] = Some(*rect);
                }
            }
        }
        for cz in 0..LOD_TILE_CELLS {
            for cx in 0..LOD_TILE_CELLS {
                assert_eq!(
                    cover[cz as usize][cx as usize].is_some(),
                    surface.drawn(cx, cz),
                    "cell ({cx}, {cz}) coverage"
                );
            }
        }
    }

    #[test]
    fn a_flat_cell_grid_merges_into_one_top_rectangle() {
        let mut rects = Vec::new();
        merge_tops(&flat_surface(12), &mut rects);
        assert_eq!(
            rects,
            vec![TopRect {
                origin: [0, 0],
                extent: [LOD_TILE_CELLS, LOD_TILE_CELLS],
                height: 12,
                colour: [0.1, 0.2, 0.3],
                tone: [0; 3],
            }],
            "a flat tile is one quad whatever its cell count"
        );
    }

    #[test]
    fn a_merged_top_carries_the_mean_tone_of_its_cells() {
        let mut surface = flat_surface(6);
        // Two tones, two cells each along x, over rows 0..4: the aggregate is
        // their mean, not the tone of the cell the greedy scan started on.
        for cz in 0..4 {
            for cx in 0..4 {
                let tone = if cx < 2 { [8, -4, 0] } else { [-2, 10, 6] };
                let mut only = cell(6, [0.1, 0.2, 0.3]);
                only.tone = tone;
                set(&mut surface, cx, cz, only);
            }
            // The rest of the row keeps its zero tone, so a 4x4 aggregate is
            // exactly the two tones it covers.
        }
        assert_eq!(
            merged_tone(&surface, 0, 0, 4, 4),
            [(8 - 2) / 2, (10 - 4) / 2, 6 / 2],
            "the mean over the merged cells"
        );
        // A whole-tile merge of one tone carries that tone unchanged, and the
        // tint reaches the emitted colour.
        let mut uniform = flat_surface(6);
        for cz in -TILE_HALO..=LOD_TILE_CELLS {
            for cx in -TILE_HALO..=LOD_TILE_CELLS {
                let mut only = cell(6, [0.1, 0.2, 0.3]);
                only.tone = [8, -4, 16];
                set(&mut uniform, cx, cz, only);
            }
        }
        let mut rects = Vec::new();
        merge_tops(&uniform, &mut rects);
        assert_eq!(rects.len(), 1, "one height, one base colour, one tone");
        assert_eq!(rects[0].tone, [8, -4, 16]);
        assert_ne!(
            material::tint(rects[0].colour, rects[0].tone),
            rects[0].colour,
            "the aggregate reaches the quad"
        );
    }

    #[test]
    fn tops_merge_runs_but_not_across_a_height_or_colour_change() {
        let mut surface = flat_surface(4);
        set(&mut surface, 5, 0, cell(9, [0.1, 0.2, 0.3]));
        set(&mut surface, 9, 0, cell(4, [0.9, 0.9, 0.9]));
        let mut rects = Vec::new();
        merge_tops(&surface, &mut rects);
        assert_tops_cover(&surface, &rects);
        // Two single cells and the runs they cut row 0 into stay separate; the
        // rest of the tile merges column-wise, so 1024 cells cost seven quads
        // rather than 1024.
        assert_eq!(rects.len(), 7, "{rects:?}");
        assert!(rects.iter().any(|r| r.extent == [5, 32]));
    }

    #[test]
    fn equal_wall_runs_merge_and_unequal_ones_break() {
        let cell_m = 8;
        let mut surface = flat_surface(0);
        // A block five cells wide at height 3 in row 2, a gap, then three more.
        for cx in 0..5 {
            set(&mut surface, cx, 2, cell(3, [0.1, 0.2, 0.3]));
        }
        for cx in 6..9 {
            set(&mut surface, cx, 2, cell(3, [0.1, 0.2, 0.3]));
        }
        let mut runs = Vec::new();
        merge_walls(&surface, cell_m, &mut runs);
        let north: Vec<_> = runs
            .iter()
            .filter(|run| run.normal == [0, 0, -1] && run.plane == 2 && run.start < 9)
            .collect();
        assert_eq!(
            north,
            vec![
                &WallRun {
                    normal: [0, 0, -1],
                    plane: 2,
                    start: 0,
                    len: 5,
                    top: 3,
                    foot: 0,
                    colour: [0.1, 0.2, 0.3],
                    tone: [0; 3],
                },
                &WallRun {
                    normal: [0, 0, -1],
                    plane: 2,
                    start: 6,
                    len: 3,
                    top: 3,
                    foot: 0,
                    colour: [0.1, 0.2, 0.3],
                    tone: [0; 3],
                },
            ],
            "two runs, split where the block is interrupted: {runs:?}"
        );
        // The same block faces water on its east side: one run along z.
        let east: Vec<_> = runs
            .iter()
            .filter(|run| run.normal == [1, 0, 0] && run.plane == 5)
            .collect();
        assert_eq!(east.len(), 1, "{runs:?}");
        assert_eq!(
            (east[0].start, east[0].len, east[0].top, east[0].foot),
            (2, 1, 3, 0)
        );
        assert_eq!((east[0].plane, east[0].normal), (5, [1, 0, 0]));
    }

    #[test]
    fn a_wall_over_a_filtered_edge_hangs_below_its_own_top() {
        let cell_m = 2;
        const EAST_HALO: usize = (LOD_TILE_CELLS + 2 * TILE_HALO - 1) as usize;

        // An undrawn neighbour across the east face of cell (31, 0), lower.
        let mut surface = flat_surface(10);
        surface.drawn[TILE_HALO as usize][EAST_HALO] = false;
        surface.sample[TILE_HALO as usize][EAST_HALO] = cell(0, [0.0, 0.0, 0.0]);
        let spec = wall_spec(&surface, cell_m, 31, 0, [1, 0]).expect("a filtered edge still walls");
        assert_eq!(spec.0, 10, "the wall starts at the cell's own top");
        assert_eq!(
            spec.1, 0,
            "and reaches the neighbour's ground when that is lower"
        );

        // A filtered neighbour that stands higher: the wall still hangs the
        // skirt depth below the cell top, so the seam cannot open.
        let mut surface = flat_surface(10);
        surface.drawn[TILE_HALO as usize][EAST_HALO] = false;
        surface.sample[TILE_HALO as usize][EAST_HALO] = cell(40, [0.0, 0.0, 0.0]);
        let spec = wall_spec(&surface, cell_m, 31, 0, [1, 0]).unwrap();
        assert_eq!(
            (spec.0, spec.1),
            (10, 10 - FILTER_WALL_CELLS * cell_m),
            "a higher neighbour is covered by the skirt depth"
        );

        // The wall wears the higher cell's own sub-surface layer, and bare rock
        // when the step is deeper than that layer.
        let mut deep = flat_surface(10);
        set(
            &mut deep,
            3,
            3,
            CellSample {
                height: 10,
                colour: [0.1, 0.2, 0.3],
                tone: [0; 3],
                sub_colour: [0.4, 0.3, 0.2],
                sub_tone: [0; 3],
                rock_tone: [0; 3],
                sub_depth: 4,
            },
        );
        set(&mut deep, 4, 3, cell(8, [0.1, 0.2, 0.3]));
        assert_eq!(
            wall_spec(&deep, cell_m, 3, 3, [1, 0]),
            Some((10, 8, [0.4, 0.3, 0.2], [0; 3])),
            "a step inside the sub-surface layer shows that layer"
        );
        set(&mut deep, 4, 3, cell(2, [0.1, 0.2, 0.3]));
        assert_eq!(
            wall_spec(&deep, cell_m, 3, 3, [1, 0]),
            Some((10, 2, material::color(material::STONE), [0; 3])),
            "a step deeper than the sub-surface layer is bare rock"
        );

        // Two drawn cells never double a wall: only the higher one emits it.
        let mut low = flat_surface(10);
        set(&mut low, 4, 3, cell(2, [0.1, 0.2, 0.3]));
        assert_eq!(
            wall_spec(&low, cell_m, 3, 3, [1, 0]),
            Some((10, 2, [0.1, 0.2, 0.3], [0; 3])),
            "the higher cell owns the face"
        );
        assert_eq!(
            wall_spec(&low, cell_m, 4, 3, [-1, 0]),
            None,
            "the lower cell draws nothing there"
        );
        assert_eq!(
            wall_spec(&flat_surface(10), cell_m, 3, 3, [1, 0]),
            None,
            "equal neighbours need no wall"
        );
    }

    #[test]
    fn quantisation_floors_toward_the_ground() {
        assert_eq!(quantise_height(7, 4), 4);
        assert_eq!(quantise_height(4, 4), 4);
        assert_eq!(quantise_height(0, 8), 0);
        assert_eq!(quantise_height(-1, 4), -4);
        assert_eq!(quantise_height(3, 1), 3);
    }

    #[test]
    fn the_step_table_follows_the_ring_derivation() {
        // The same integer arithmetic the table's doc comment describes: 2 px at
        // 65 degrees over a 1440 px viewport is 1.576 mm per metre of distance,
        // rounded to the nearest power of two metres and never below 1 m.
        fn derived_step_m(half_extent_m: i32) -> i32 {
            let needed_mm = (half_extent_m as i64 * 1576 / 1000).max(1000);
            let mut step = 1000i64;
            while (2 * step - needed_mm).abs() < (step - needed_mm).abs() {
                step *= 2;
            }
            (step / 1000) as i32
        }
        assert_eq!(LOD_STEP_M, [1, 1, 1, 2, 4, 8, 16]);
        for config in LANDSCAPE_RINGS {
            assert_eq!(
                lod_step_m(config.level),
                derived_step_m(config.half_extent),
                "level {} over {} m",
                config.level,
                config.half_extent
            );
        }
        assert_eq!(lod_step_m(0), 1, "the finest level is exact");
    }

    #[test]
    fn a_tile_filter_agrees_with_the_mesher_about_the_cell_it_draws() {
        // A cell-sized window inside a tile: the filter's own cell is drawn and
        // the hole's cell is not, at every level the rings use.
        for level in 0..=MAX_LOD_LEVEL {
            let cell = lod_cell_m(level);
            let filter = TileFilter {
                hole: Some(Clip {
                    min: [0, 0],
                    max: [cell, cell],
                }),
                bound: Some(Clip {
                    min: [-cell, -cell],
                    max: [cell, cell],
                }),
                bed: None,
            };
            assert!(
                filter.covers_point(level, -1, -1),
                "inside the bound and outside the hole is drawn at level {level}"
            );
            assert!(
                !filter.covers_point(level, 0, 0),
                "the hole's own cell is not drawn at level {level}"
            );
            assert!(
                !filter.covers_point(level, cell, cell),
                "the bound's far edge is not drawn at level {level}"
            );
        }
        // A point anywhere in a cell tests the cell's centre, not the point.
        let level = 1;
        let filter = TileFilter {
            hole: Some(Clip {
                min: [1, 1],
                max: [2, 2],
            }),
            bound: None,
            bed: None,
        };
        assert!(
            !filter.covers_point(level, 0, 0),
            "the cell centre (1, 1) is inside"
        );
        assert!(
            filter.covers_point(level, 2, 2),
            "the cell centre (3, 3) is outside"
        );
    }

    #[test]
    fn ring_squares_nest_and_align_to_the_next_cell() {
        for eye in [
            [0.0, 0.0, 0.0],
            [37.5, 12.0, -913.25],
            [-5000.0, 0.0, 4096.0],
        ] {
            let plan = ring_plan(eye, fine_clip(eye), &LANDSCAPE_RINGS);
            let mut previous: Option<Clip> = None;
            for config in LANDSCAPE_RINGS {
                let bound = plan
                    .iter()
                    .find(|tile| tile.level == config.level)
                    .and_then(|tile| tile.filter.bound)
                    .expect("every ring emits tiles");
                let cell = lod_cell_m(config.level);
                for edge in [bound.min[0], bound.min[1], bound.max[0], bound.max[1]] {
                    assert_eq!(edge.rem_euclid(config.tile_size_m()), 0, "{edge}");
                    assert_eq!(edge.rem_euclid(cell), 0, "{edge}");
                }
                if let Some(inner) = previous {
                    assert!(bound.min[0] <= inner.min[0] && bound.max[0] >= inner.max[0]);
                    assert!(bound.min[1] <= inner.min[1] && bound.max[1] >= inner.max[1]);
                    // The inner square's edges must fall on this ring's cell grid.
                    for edge in [inner.min[0], inner.min[1], inner.max[0], inner.max[1]] {
                        assert_eq!(edge.rem_euclid(cell), 0, "hole edge {edge} at cell {cell}");
                    }
                }
                previous = Some(bound);
            }
        }
    }

    #[test]
    fn ring_plan_reuses_its_buffer() {
        let mut plan = Vec::new();
        ring_plan_into(
            [0.0, 0.0, 0.0],
            fine_clip([0.0; 3]),
            &LANDSCAPE_RINGS,
            &mut plan,
        );
        let capacity = plan.capacity();
        let length = plan.len();
        assert!(length > 0);
        for step in 0..64 {
            let eye = [step as f32 * 13.0, 20.0, step as f32 * -7.0];
            ring_plan_into(eye, fine_clip(eye), &LANDSCAPE_RINGS, &mut plan);
            assert_eq!(
                plan.len(),
                length,
                "the plan size must not depend on the eye"
            );
        }
        assert_eq!(
            plan.capacity(),
            capacity,
            "a warm plan buffer must not grow"
        );
    }

    #[test]
    fn nonfinite_eyes_plan_like_the_origin() {
        let origin = ring_plan([0.0, 0.0, 0.0], fine_clip([0.0; 3]), &LANDSCAPE_RINGS);
        for eye in [
            [f32::NAN, 0.0, 0.0],
            [f32::INFINITY, 0.0, f32::NEG_INFINITY],
        ] {
            assert_eq!(ring_plan(eye, fine_clip(eye), &LANDSCAPE_RINGS), origin);
        }
    }

    #[test]
    fn value_noise_stays_inside_its_range() {
        for x in -300..300 {
            for z in -50..50 {
                let value = value_noise(7, SALT_HILLS, x * 3, z * 5, 64);
                assert!((0..=65535).contains(&value), "{value}");
            }
        }
    }

    #[test]
    fn fbm_is_bounded_and_varies() {
        let mut low = i32::MAX;
        let mut high = i32::MIN;
        for x in -500..500 {
            let value = fbm(11, SALT_CONTINENT, x, x / 3, 128, 4);
            assert!((0..=65535).contains(&value), "{value}");
            low = low.min(value);
            high = high.max(value);
        }
        assert!(
            high - low > 4096,
            "field must not be constant: {low}..{high}"
        );
    }

    #[test]
    fn ridged_noise_is_bounded() {
        for x in -200..200 {
            let value = ridged(3, SALT_RIDGE, x * 11, -x * 7, 96, 4);
            assert!((0..=65535).contains(&value), "{value}");
        }
    }

    #[test]
    fn ramp_clamps_and_saturates() {
        assert_eq!(ramp(0, 10, 20), 0);
        assert_eq!(ramp(15, 10, 20), 128);
        assert_eq!(ramp(999, 10, 20), 256);
        assert_eq!(ramp(-999, 10, 20), 0);
    }

    #[test]
    fn flora_cells_stay_inside_their_cell() {
        for cell in -30..30 {
            let flora = flora_cell(9, cell, -cell);
            assert!(flora.len() <= MAX_FLORA_PER_CELL);
            for site in flora.sites() {
                assert!((cell * FLORA_CELL_M..(cell + 1) * FLORA_CELL_M).contains(&site.x));
                assert!(((-cell) * FLORA_CELL_M..(-cell + 1) * FLORA_CELL_M).contains(&site.z));
                assert!((4..=12).contains(&site.scale_eighths));
                assert!(site.yaw_quarters <= 3);
            }
        }
    }

    // -- Flora planning ------------------------------------------------------

    /// The landscape sample's seed. Kept here so the planner tests exercise the
    /// world the sample actually shows.
    const SEED_UNDER_TEST: u64 = 20260913;
    const PLAINS_EYE: [f32; 3] = [0.0, 40.0, 0.0];

    fn find_biome(seed: u64, want: Biome) -> Option<[f32; 3]> {
        // A coarse sweep of the domain; the generator is a function of (x, z),
        // so the first hit is deterministic.
        for step_z in -40..40 {
            for step_x in -40..40 {
                let x = step_x * 97;
                let z = step_z * 89;
                if biome_at(seed, x, z) == want {
                    return Some([x as f32 + 0.5, 40.0, z as f32 + 0.5]);
                }
            }
        }
        None
    }

    fn plan_default(eye: [f32; 3]) -> FloraPlan {
        plan_flora(SEED_UNDER_TEST, eye, &LANDSCAPE_FLORA_TIERS, 20_000, 4_000)
    }

    #[test]
    fn flora_plans_are_deterministic_and_free_of_duplicates() {
        use std::collections::BTreeSet;
        for eye in [PLAINS_EYE, [-137.5, 12.0, 913.25], [7.0, 0.0, -3.0]] {
            let plan = plan_default(eye);
            assert_eq!(
                plan,
                plan_default(eye),
                "the plan must be a function of the eye"
            );
            // No lattice cell is visited twice, so no site is emitted twice.
            // (A single column may legitimately carry two plants of the same
            // kind: `flora_cell` rolls ground cover and flowers separately.)
            let mut cells = BTreeSet::new();
            let mut previous: Option<(i32, i32)> = None;
            for site in &plan.sites {
                let cell = (
                    site.x.div_euclid(FLORA_CELL_M),
                    site.z.div_euclid(FLORA_CELL_M),
                );
                if previous != Some(cell) {
                    assert!(cells.insert(cell), "flora cell {cell:?} was visited twice");
                    previous = Some(cell);
                }
            }
            let mut trees = BTreeSet::new();
            for tree in &plan.trees {
                assert!(trees.insert((tree.x, tree.z)), "duplicate tree {tree:?}");
            }
            for placed in plan.sites.iter().chain(&plan.trees) {
                assert!(placed.yaw_quarters <= 3);
                assert!((4..=12).contains(&placed.scale_eighths));
                assert!((placed.tier as usize) < LANDSCAPE_FLORA_TIERS.len());
                // The per-site variation is part of the plan, so the equality
                // above already covers it; these bound it and check that the
                // field is actually varied rather than a constant that would
                // make the equality trivially true.
                assert!((FLORA_MIN_VARIATION_PERCENT..=FLORA_MAX_VARIATION_PERCENT)
                    .contains(&placed.height_percent));
                assert!((FLORA_MIN_VARIATION_PERCENT..=FLORA_MAX_VARIATION_PERCENT)
                    .contains(&placed.bend_percent));
            }
        }
        // A non-finite eye folds to the origin, exactly as the ring plan does.
        assert_eq!(
            plan_default([f32::NAN, 0.0, f32::INFINITY]),
            plan_default([0.0, 0.0, 0.0])
        );
    }

    #[test]
    fn per_site_variation_is_ranged_varied_and_decorrelated() {
        use std::collections::BTreeSet;
        let seed = SEED_UNDER_TEST;
        let mut heights = BTreeSet::new();
        let mut bends = BTreeSet::new();
        let mut pairs = BTreeSet::new();
        let mut adjacent = 0usize;
        let mut same_height = 0usize;
        let mut same_both = 0usize;
        for x in -50..50 {
            for z in -50..50 {
                let height = varied_percent(seed, SALT_FLORA_HEIGHT, x, z);
                let bend = varied_percent(seed, SALT_FLORA_BEND, x, z);
                assert!(
                    (FLORA_MIN_VARIATION_PERCENT..=FLORA_MAX_VARIATION_PERCENT).contains(&height),
                    "{height} out of range"
                );
                assert!(
                    (FLORA_MIN_VARIATION_PERCENT..=FLORA_MAX_VARIATION_PERCENT).contains(&bend),
                    "{bend} out of range"
                );
                heights.insert(height);
                bends.insert(bend);
                pairs.insert((height, bend));
                for (dx, dz) in [(1, 0), (0, 1)] {
                    let other_height = varied_percent(seed, SALT_FLORA_HEIGHT, x + dx, z + dz);
                    let other_bend = varied_percent(seed, SALT_FLORA_BEND, x + dx, z + dz);
                    adjacent += 1;
                    same_height += usize::from(height == other_height);
                    same_both += usize::from(height == other_height && bend == other_bend);
                }
            }
        }
        // Every value in the range is reachable, so the field is a real
        // variation and not a clamped constant.
        assert_eq!(heights.len(), 51, "height values");
        assert_eq!(bends.len(), 51, "bend values");
        // Independence, stated as coverage: if the bend were a relabelling of
        // the height, the joint field would occupy 51 of the 51x51 cells. The
        // measured coverage over this grid is 2491 of 2601, so the two fields
        // carry independent hash bits.
        assert!(
            pairs.len() >= 2400,
            "height and bend are not independent: {} of 2601 pairs",
            pairs.len()
        );
        // Adjacent columns: measured over this grid, 385 of the 20000 adjacent
        // pairs (1.9%) share a height and 10 (0.05%) share both fields, which is
        // the birthday rate for 51 values. The margins below guard a collapse
        // into a constant, not a collision, which no hash can rule out.
        assert!(
            same_height * 33 < adjacent,
            "{same_height} of {adjacent} neighbours share a height"
        );
        assert!(
            same_both * 500 < adjacent,
            "{same_both} of {adjacent} neighbours share both fields"
        );
        // The fields are integer arithmetic on the column, so they are stable
        // across calls and across targets.
        assert_eq!(
            varied_percent(seed, SALT_FLORA_HEIGHT, -137, 913),
            varied_percent(seed, SALT_FLORA_HEIGHT, -137, 913)
        );
        assert_ne!(
            varied_percent(seed, SALT_FLORA_HEIGHT, 0, 0),
            varied_percent(seed, SALT_FLORA_HEIGHT, 1, 0)
        );
    }

    #[test]
    fn the_density_profile_is_continuous_and_ends_at_zero() {
        // The profile the planner uses, sampled on the lattice it uses. It must
        // be flat inside the first band, monotone, and reach zero at the last
        // radius without a step anywhere on the way.
        let first = LANDSCAPE_FLORA_TIERS[0];
        let outer = LANDSCAPE_FLORA_TIERS
            .iter()
            .map(|tier| tier.radius_m)
            .max()
            .unwrap();
        assert_eq!(
            density_percent_at(&LANDSCAPE_FLORA_TIERS, 0),
            u32::from(first.density_percent)
        );
        assert_eq!(
            density_percent_at(&LANDSCAPE_FLORA_TIERS, first.radius_m),
            u32::from(first.density_percent)
        );
        assert_eq!(density_percent_at(&LANDSCAPE_FLORA_TIERS, outer), 0);
        assert_eq!(density_percent_at(&LANDSCAPE_FLORA_TIERS, 400), 0);
        let mut previous = 100;
        for distance in 0..=outer {
            let density = density_percent_at(&LANDSCAPE_FLORA_TIERS, distance);
            assert!(
                density <= previous,
                "density rose at {distance} m: {density} after {previous}"
            );
            assert!(
                previous - density <= 12,
                "density stepped {previous} -> {density} in one metre at {distance} m"
            );
            previous = density;
        }
    }

    #[test]
    fn the_planner_keeps_a_continuous_fraction_of_lattice_cells() {
        // The measured survival, shell by shell, from the planner's own
        // predicate. Measuring *cells* rather than sites avoids a biome
        // gradient across the window reading as a density profile, and it is
        // the quantity the profile defines: the fraction of 2 m cells the plan
        // visits. The integer `keep_every` this replaced fell by 8x at the tier
        // edge; the smoothstep fade changes by at most its own slope.
        const SHELL_M: i32 = 4;
        let outer = LANDSCAPE_FLORA_TIERS
            .iter()
            .map(|tier| tier.radius_m)
            .max()
            .unwrap();
        let shells = (outer / SHELL_M) as usize;
        assert_eq!(outer % SHELL_M, 0);
        let mut kept = vec![0usize; shells + 1];
        let mut total = vec![0usize; shells + 1];
        // Every lattice cell whose origin distance falls in each shell, over the
        // whole outer square.
        for cell_z in -outer.div_euclid(FLORA_CELL_M)..=outer.div_euclid(FLORA_CELL_M) {
            for cell_x in -outer.div_euclid(FLORA_CELL_M)..=outer.div_euclid(FLORA_CELL_M) {
                let origin_x = cell_x * FLORA_CELL_M;
                let origin_z = cell_z * FLORA_CELL_M;
                let distance = chebyshev(origin_x, origin_z, 0, 0);
                let shell = (distance / SHELL_M) as usize;
                total[shell] += 1;
                if keeps_flora_cell(
                    SEED_UNDER_TEST,
                    [cell_x, cell_z],
                    &LANDSCAPE_FLORA_TIERS,
                    distance,
                ) {
                    kept[shell] += 1;
                }
            }
        }
        let fraction = |shell: usize| -> f64 {
            if total[shell] == 0 {
                0.0
            } else {
                kept[shell] as f64 / total[shell] as f64
            }
        };
        // The plateau is full and the outer shell is empty: the profile starts
        // dense and ends at zero rather than at a fence.
        assert_eq!(fraction(0), 1.0, "the eye's own cells are never thinned");
        let beyond = fraction(shells);
        assert_eq!(beyond, 0.0, "nothing survives past the outer radius");
        // Monotone, and no shell changes the density by a step: the smoothstep's
        // steepest metre is 9.4%, so a four-metre shell cannot move more than
        // 0.38. The tier boundary in particular is mid-plateau.
        let mut previous = fraction(0);
        for shell in 1..=shells {
            let current = fraction(shell);
            assert!(
                current <= previous,
                "density rose at shell {shell}: {current:.3} after {previous:.3}"
            );
            assert!(
                previous - current <= 0.42,
                "kept fraction stepped {previous:.3} -> {current:.3} at {} m",
                shell * SHELL_M as usize
            );
            previous = current;
        }
        let boundary = (LANDSCAPE_FLORA_TIERS[0].radius_m / SHELL_M) as usize;
        assert!(
            fraction(boundary) >= 0.9,
            "the {} m band boundary thinned to {:.3}",
            LANDSCAPE_FLORA_TIERS[0].radius_m,
            fraction(boundary)
        );
        // And the plan itself keeps the near field: ground cover exists within
        // 16 m of the eye and inside the cell the eye stands in, which is what
        // "vegetation in the foreground" means as a number.
        let plan = plan_default(PLAINS_EYE);
        let near = plan
            .sites
            .iter()
            .filter(|site| chebyshev(site.x, site.z, 0, 0) <= 16)
            .count();
        assert!(near > 0, "the plan drew nothing within 16 m of the eye");
        let stand = [0i32.div_euclid(FLORA_CELL_M), 0i32.div_euclid(FLORA_CELL_M)];
        assert!(
            plan.sites.iter().any(|site| {
                site.x.div_euclid(FLORA_CELL_M) == stand[0]
                    && site.z.div_euclid(FLORA_CELL_M) == stand[1]
            }),
            "no plant stands in the cell under the eye"
        );
    }

    #[test]
    fn caps_refuse_placements_and_report_them() {
        let full = plan_default(PLAINS_EYE);
        assert_eq!(full.dropped, 0, "the generous caps must not bite");
        assert!(full.sites.len() > 500, "{} sites", full.sites.len());
        let capped = plan_flora(SEED_UNDER_TEST, PLAINS_EYE, &LANDSCAPE_FLORA_TIERS, 100, 7);
        assert_eq!(capped.sites.len(), 100);
        assert_eq!(capped.trees.len(), full.trees.len().min(7));
        assert_eq!(
            capped.dropped,
            (full.sites.len() - 100) + (full.trees.len() - capped.trees.len()),
            "every refused placement must be counted"
        );
        // Capping drops the tail; it does not reshuffle the kept prefix.
        assert_eq!(capped.sites, full.sites[..100]);
    }

    #[test]
    fn the_plan_reuses_its_buffers() {
        let mut plan = FloraPlan::default();
        // Warm the buffers on a few eyes first; capacity must then hold.
        for step in 0..6 {
            let eye = [step as f32 * 37.0, 20.0, step as f32 * -53.0];
            plan_flora_into(
                SEED_UNDER_TEST,
                eye,
                &LANDSCAPE_FLORA_TIERS,
                20_000,
                4_000,
                &mut plan,
            );
        }
        let sites = plan.sites.capacity();
        let trees = plan.trees.capacity();
        for step in 0..6 {
            let eye = [step as f32 * -17.0, 20.0, step as f32 * 29.0];
            plan_flora_into(
                SEED_UNDER_TEST,
                eye,
                &LANDSCAPE_FLORA_TIERS,
                sites,
                trees,
                &mut plan,
            );
            assert_eq!(plan.sites.capacity(), sites, "a warm site buffer grew");
            assert_eq!(plan.trees.capacity(), trees, "a warm tree buffer grew");
        }
    }

    #[test]
    fn trees_stay_inside_their_documented_radius() {
        for eye in [PLAINS_EYE, [-903.5, 20.0, 411.25]] {
            let plan = plan_default(eye);
            let eye_x = eye_metre(eye[0]);
            let eye_z = eye_metre(eye[2]);
            for tree in &plan.trees {
                assert!(
                    chebyshev(tree.x, tree.z, eye_x, eye_z) <= LANDSCAPE_TREE_RADIUS_M,
                    "tree {tree:?} escaped the plan radius"
                );
                assert!(matches!(
                    tree.kind,
                    FloraKind::TreeBroadleaf | FloraKind::TreeConifer
                ));
            }
            for site in &plan.sites {
                assert!(
                    !matches!(site.kind, FloraKind::TreeBroadleaf | FloraKind::TreeConifer),
                    "a tree reached the ground-cover list"
                );
                assert!(chebyshev(site.x, site.z, eye_x, eye_z) <= 96 + FLORA_CELL_M);
            }
        }
    }

    #[test]
    fn biomes_grow_what_they_should() {
        let seed = SEED_UNDER_TEST;
        let plains = find_biome(seed, Biome::Plains).expect("the world has plains");
        let plan = plan_flora(seed, plains, &LANDSCAPE_FLORA_TIERS, 20_000, 4_000);
        assert!(
            plan.sites
                .iter()
                .filter(|s| s.kind == FloraKind::GrassTuft)
                .count()
                > 100,
            "plains must be grassy"
        );
        assert!(plan.sites.iter().any(|s| matches!(
            s.kind,
            FloraKind::FlowerRed | FloraKind::FlowerWhite | FloraKind::FlowerYellow
        )));

        let desert = find_biome(seed, Biome::Desert).expect("the world has desert");
        let plan = plan_flora(seed, desert, &LANDSCAPE_FLORA_TIERS, 20_000, 4_000);
        assert!(
            plan.sites.iter().any(|s| s.kind == FloraKind::Cactus),
            "desert must grow cacti"
        );

        let forest = find_biome(seed, Biome::Forest).expect("the world has forest");
        let plan = plan_flora(seed, forest, &LANDSCAPE_FLORA_TIERS, 20_000, 4_000);
        assert!(
            plan.trees.len() > 200,
            "a forest must read as forest: {} trees",
            plan.trees.len()
        );
    }

    /// Structural placement budget for one 16 m square at full density: 8x8
    /// flora cells of at most [`MAX_FLORA_PER_CELL`] sites plus 2x2 tree cells
    /// of one tree each.
    const MAX_PLACED_PER_16M_SQUARE: usize = 64 * MAX_FLORA_PER_CELL + 4;

    #[test]
    fn per_16m_square_density_stays_inside_its_budget() {
        use std::collections::BTreeMap;
        let plains = find_biome(SEED_UNDER_TEST, Biome::Plains).expect("the world has plains");
        let plan = plan_flora(
            SEED_UNDER_TEST,
            plains,
            &LANDSCAPE_FLORA_TIERS,
            20_000,
            4_000,
        );
        let mut squares: BTreeMap<(i32, i32), usize> = BTreeMap::new();
        for placed in plan.sites.iter().chain(&plan.trees) {
            *squares
                .entry((placed.x.div_euclid(16), placed.z.div_euclid(16)))
                .or_default() += 1;
        }
        let worst = *squares.values().max().expect("a plains plan is not empty");
        assert!(
            worst <= MAX_PLACED_PER_16M_SQUARE,
            "{worst} placements in one 16 m square exceeds {MAX_PLACED_PER_16M_SQUARE}"
        );
        // Full-density squares must still read as dense ground cover.
        assert!(
            worst >= 64,
            "the densest 16 m square holds only {worst} plants"
        );
    }

    #[test]
    fn emitted_faces_are_flat_and_wound_toward_their_normal() {
        let colour = [0.1, 0.2, 0.3];
        let mut mesh = Mesh::default();
        emit_top(
            &mut mesh,
            TopRect {
                origin: [1, 2],
                extent: [3, 4],
                height: 7,
                colour,
                tone: [0; 3],
            },
            [0, 0],
            4,
        );
        for normal in [[-1, 0, 0], [1, 0, 0], [0, 0, -1], [0, 0, 1]] {
            emit_wall(
                &mut mesh,
                WallRun {
                    normal,
                    plane: 2,
                    start: 1,
                    len: 3,
                    top: 7,
                    foot: 3,
                    colour,
                    tone: [0; 3],
                },
                [0, 0],
                4,
            );
        }
        assert_eq!(
            mesh.vertices.len(),
            5 * 4,
            "one quad per face, four vertices"
        );
        for (index, quad) in mesh.indices.chunks(6).enumerate() {
            let vertices: Vec<Vertex> = quad.iter().map(|&i| mesh.vertices[i as usize]).collect();
            let expected = vertices[0].normal;
            for vertex in &vertices {
                assert_eq!(vertex.normal, expected);
                assert_eq!(vertex.color, colour);
            }
            let low = vertices
                .iter()
                .map(|v| v.position[1])
                .fold(f32::INFINITY, f32::min);
            let high = vertices
                .iter()
                .map(|v| v.position[1])
                .fold(f32::NEG_INFINITY, f32::max);
            if index == 0 {
                assert_eq!((low, high), (7.0, 7.0), "the top rectangle is flat");
            } else {
                assert_eq!((low, high), (3.0, 7.0), "the wall spans its own step");
            }
            let cross = cross_product(&vertices);
            assert_eq!(
                cross, expected,
                "winding must point along the face normal: {vertices:?}"
            );
        }
    }

    /// `cross(p1 - p0, p2 - p0)` of a quad's first three corners, as signs: the
    /// renderer culls back faces, so this is the direction the face is visible
    /// from.
    fn cross_product(vertices: &[Vertex]) -> [f32; 3] {
        let u = [
            vertices[1].position[0] - vertices[0].position[0],
            vertices[1].position[1] - vertices[0].position[1],
            vertices[1].position[2] - vertices[0].position[2],
        ];
        let v = [
            vertices[2].position[0] - vertices[0].position[0],
            vertices[2].position[1] - vertices[0].position[1],
            vertices[2].position[2] - vertices[0].position[2],
        ];
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        assert!(length > 0.0, "a degenerate face has no facing");
        cross.map(|value| value / length)
    }
}
