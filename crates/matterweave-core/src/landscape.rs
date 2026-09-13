//! Versioned procedural landscape: biome terrain, relief, water levels and flora.
//!
//! This is a pure, deterministic function of `(seed, coordinate)`. It allocates
//! nothing on the sampling path, reads no world state and has no platform
//! dependency, so the same seed and generator version produce byte-identical
//! columns on every target. All arithmetic is integer or fixed point; no
//! `f32`/`f64` operation participates in a result. That is deliberate: a stored
//! world must regenerate exactly what it was saved from, and ARM64 and x86-64
//! must not drift.
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
//! distance rings. Tile vertices sample the generator exactly at their own
//! coordinate, so a tile's edge matches the finer level's edge at the same metre
//! coordinate and no crack can open at a level boundary. Relief narrower than a
//! coarse cell is not represented at that level; [`lod_sample`] provides the
//! conservative maximum for tools and silhouette checks.

use crate::hash;
use crate::material;
use crate::mesh::{Mesh, Vertex};

/// Identity of this generator's output. Bump for any change to the columns,
/// materials or flora population it produces.
pub const LANDSCAPE_GENERATOR_VERSION: u32 = 1;

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
    pub fn grass_density(self) -> u8 {
        match self {
            Biome::Ocean => 0,
            Biome::Beach | Biome::Desert | Biome::Snow => 2,
            Biome::Mountain | Biome::Tundra => 10,
            Biome::Hills => 40,
            Biome::Plains => 52,
            Biome::Forest => 44,
            Biome::Swamp => 34,
        }
    }

    /// Flower pressure in `0..=64` per flora slot.
    pub fn flower_density(self) -> u8 {
        match self {
            Biome::Ocean | Biome::Snow => 0,
            Biome::Desert => 4,
            Biome::Beach => 3,
            Biome::Mountain | Biome::Tundra => 8,
            Biome::Hills => 14,
            Biome::Plains => 22,
            Biome::Forest => 12,
            Biome::Swamp => 10,
        }
    }

    /// Tree pressure in `0..=64` per 8 m cell.
    pub fn tree_density(self) -> u8 {
        match self {
            Biome::Forest => 44,
            Biome::Swamp => 22,
            Biome::Plains | Biome::Hills => 6,
            Biome::Tundra => 3,
            Biome::Beach | Biome::Desert | Biome::Mountain | Biome::Snow | Biome::Ocean => 0,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileFilter {
    /// Cells inside this square are omitted; a finer level covers them.
    pub hole: Option<Clip>,
    /// Cells outside this square are omitted; a coarser level covers them.
    pub bound: Option<Clip>,
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
    // to the 0..112 m mountain range.
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
    // Snow line falls with temperature: about 116 m cold, 74 m warm.
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
        // Ore pockets inside rock layers add variety without changing the surface.
        if (column.sub_surface == material::STONE || column.sub_surface == material::GRAVEL)
            && ((hash3(seed ^ SALT_ORE, x, z) >> 40) & 0x3F) < 2
        {
            return material::MINERAL;
        }
        return column.sub_surface;
    }
    // Deep rock keeps a deterministic vein pattern so deep cuts are not flat.
    if (hash3(seed ^ SALT_ROCK, x * 7 + y, z) >> 44) & 0x3F < 3 {
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
                scale_eighths: (4 + ((roll >> 16) & 7)) as u8,
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
        scale_eighths: (5 + ((roll >> 16) & 6)) as u8,
    })
}

// -- Distance levels ---------------------------------------------------------

/// Coarse cell edge in metres for a level: level 0 is one metre.
pub fn lod_cell_m(level: u32) -> i32 {
    1i32 << level.min(MAX_LOD_LEVEL)
}

/// Surface sample at one coarse grid vertex, at its exact metre coordinate.
///
/// Exact sampling (rather than a cell maximum) is what keeps a tile's edge
/// identical to the finer level's edge at the same coordinate, so no crack and no
/// lip can open at a level boundary. Relief narrower than the level's cell is not
/// represented at that level; [`lod_sample`] provides the conservative maximum for
/// silhouette checks.
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
/// Vertices are world-space metres at `1 << level` metre spacing. Cells whose
/// centre lies in `filter.hole`, or outside `filter.bound`, are omitted and every
/// boundary the mesh stops at gets a skirt, so a nested distance ring cannot
/// show daylight and cannot overlap the level beneath it.
pub fn lod_tile_mesh(seed: u64, level: u32, key: [i32; 2], filter: TileFilter) -> Mesh {
    let level = level.min(MAX_LOD_LEVEL);
    let cell_m = lod_cell_m(level);
    let edge = LOD_TILE_CELLS + 1;
    let mut mesh = Mesh::default();
    let mut included = [[false; LOD_TILE_CELLS as usize]; LOD_TILE_CELLS as usize];
    let mut any = false;
    for cz in 0..LOD_TILE_CELLS {
        for cx in 0..LOD_TILE_CELLS {
            let centre = [
                (key[0] * LOD_TILE_CELLS + cx) * cell_m + cell_m / 2,
                (key[1] * LOD_TILE_CELLS + cz) * cell_m + cell_m / 2,
            ];
            let in_hole = filter.hole.is_some_and(|hole| hole.contains_centre(centre));
            let in_bound = filter
                .bound
                .is_none_or(|bound| bound.contains_centre(centre));
            included[cz as usize][cx as usize] = !in_hole && in_bound;
            any |= included[cz as usize][cx as usize];
        }
    }
    if !any {
        // No cells: an empty mesh is the correct representation, not a failure.
        return mesh;
    }

    // One height sample per grid vertex, cached so a shared corner is sampled once.
    let mut heights = [[0i32; LOD_TILE_CELLS as usize + 1]; LOD_TILE_CELLS as usize + 1];
    let mut colours = [[[0.0f32; 3]; LOD_TILE_CELLS as usize + 1]; LOD_TILE_CELLS as usize + 1];
    let mut wet = [[false; LOD_TILE_CELLS as usize + 1]; LOD_TILE_CELLS as usize + 1];
    for gz in 0..edge {
        for gx in 0..edge {
            let x = (key[0] * LOD_TILE_CELLS + gx) * cell_m;
            let z = (key[1] * LOD_TILE_CELLS + gz) * cell_m;
            let sample = lod_vertex(seed, x, z);
            heights[gz as usize][gx as usize] = sample.height;
            colours[gz as usize][gx as usize] = material::color(sample.surface);
            wet[gz as usize][gx as usize] = sample.flooded;
        }
    }

    let height_at = |gx: i32, gz: i32| -> i32 {
        heights[gz.clamp(0, LOD_TILE_CELLS) as usize][gx.clamp(0, LOD_TILE_CELLS) as usize]
    };
    let normal_at = |gx: i32, gz: i32| -> [f32; 3] {
        // Central difference over the (clamped) grid, so interior vertices shade
        // smoothly and edge vertices do not spike.
        let dx = (height_at(gx + 1, gz) - height_at(gx - 1, gz)) as f32;
        let dz = (height_at(gx, gz + 1) - height_at(gx, gz - 1)) as f32;
        let scale = 2.0 * cell_m as f32;
        normalize_normal([-dx / scale, 1.0, -dz / scale])
    };

    for cz in 0..LOD_TILE_CELLS {
        for cx in 0..LOD_TILE_CELLS {
            if !included[cz as usize][cx as usize] {
                continue;
            }
            let x0 = (key[0] * LOD_TILE_CELLS + cx) * cell_m;
            let z0 = (key[1] * LOD_TILE_CELLS + cz) * cell_m;
            let x1 = x0 + cell_m;
            let z1 = z0 + cell_m;
            let base = mesh.vertices.len() as u32;
            // Winding follows the core mesher: cross(U, V) points outward (+Y),
            // with U = +x and V = -z.
            let corners = [
                ([x0, heights[cz as usize][cx as usize], z1], (cx, cz + 1)),
                (
                    [x1, heights[cz as usize][cx as usize + 1], z1],
                    (cx + 1, cz + 1),
                ),
                (
                    [x1, heights[cz as usize + 1][cx as usize + 1], z0],
                    (cx + 1, cz),
                ),
                ([x0, heights[cz as usize + 1][cx as usize], z0], (cx, cz)),
            ];
            for (position, (gx, gz)) in corners {
                mesh.vertices.push(Vertex {
                    position: [position[0] as f32, position[1] as f32, position[2] as f32],
                    normal: normal_at(gx, gz),
                    // Flooded cells use the water colour even when the surface
                    // sample belongs to the shore beneath them.
                    color: if wet[gz as usize][gx as usize] {
                        material::color(material::WATER)
                    } else {
                        colours[gz as usize][gx as usize]
                    },
                });
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    add_skirts(&mut mesh, &heights, &included, key, cell_m);
    mesh
}

fn normalize_normal(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length <= f32::EPSILON || !length.is_finite() {
        [0.0, 1.0, 0.0]
    } else {
        [v[0] / length, v[1] / length, v[2] / length]
    }
}

/// Drop a vertical wall wherever the included cells stop, so a coarser
/// neighbour, a ring bound or a clip boundary cannot expose a gap.
fn add_skirts(
    mesh: &mut Mesh,
    heights: &[[i32; LOD_TILE_CELLS as usize + 1]],
    included: &[[bool; LOD_TILE_CELLS as usize]],
    key: [i32; 2],
    cell_m: i32,
) {
    let depth = (cell_m * 2).max(4);
    let colour = material::color(material::STONE);
    for cz in 0..LOD_TILE_CELLS {
        for cx in 0..LOD_TILE_CELLS {
            if !included[cz as usize][cx as usize] {
                continue;
            }
            let x0 = (key[0] * LOD_TILE_CELLS + cx) * cell_m;
            let z0 = (key[1] * LOD_TILE_CELLS + cz) * cell_m;
            let x1 = x0 + cell_m;
            let z1 = z0 + cell_m;
            let h00 = heights[cz as usize][cx as usize];
            let h10 = heights[cz as usize][cx as usize + 1];
            let h01 = heights[cz as usize + 1][cx as usize];
            let h11 = heights[cz as usize + 1][cx as usize + 1];
            // North, east, south, west edges in outward order.
            let edges = [
                ((cx, cz - 1), ([x0, h00, z0], [x1, h10, z0])),
                ((cx + 1, cz), ([x1, h10, z0], [x1, h11, z1])),
                ((cx, cz + 1), ([x1, h11, z1], [x0, h01, z1])),
                ((cx - 1, cz), ([x0, h01, z1], [x0, h00, z0])),
            ];
            for ((nx, nz), (a, b)) in edges {
                let open = nx < 0
                    || nz < 0
                    || nx >= LOD_TILE_CELLS
                    || nz >= LOD_TILE_CELLS
                    || !included[nz as usize][nx as usize];
                if !open {
                    continue;
                }
                let base = mesh.vertices.len() as u32;
                // cross(b - a, down) points away from the covered cell for every
                // edge in this outward-ordered traversal.
                for position in [a, b, [b[0], b[1] - depth, b[2]], [a[0], a[1] - depth, a[2]]] {
                    mesh.vertices.push(Vertex {
                        position: position.map(|v| v as f32),
                        normal: [0.0, 1.0, 0.0],
                        color: colour,
                    });
                }
                mesh.indices.extend_from_slice(&[
                    base,
                    base + 1,
                    base + 2,
                    base,
                    base + 2,
                    base + 3,
                ]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn tile_normals_are_unit_or_up() {
        let normal = normalize_normal([0.0, 0.0, 0.0]);
        assert_eq!(normal, [0.0, 1.0, 0.0]);
        let normal = normalize_normal([1.0, 1.0, 1.0]);
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        assert!((length - 1.0).abs() < 1.0e-5);
    }
}
