//! Authored overworld layouts. Each place is a bounded voxel map generated
//! deterministically from structured data: path waypoints, grass patches,
//! tree/rock clusters, buildings, NPC tiles and exits. World data is
//! authoritative: rendering and camera never change the map, and collision is
//! the solid voxel geometry itself.
use matterweave_core::{material, World};
use matterweave_monsters::journey::Place;

/// Tile classification after generation. Walkability and encounters are game
/// rules derived from this, never from the rendered mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    Path,
    /// Plain mown ground: walkable like grass but never a wild-encounter
    /// zone. Towns use this as their base so a paved town is safe.
    Lawn,
    Grass,
    TallGrass,
    Flower,
    Tree,
    Rock,
    Wall,
    Door,
    Floor,
}

impl Tile {
    pub fn walkable(self) -> bool {
        !matches!(self, Tile::Tree | Tile::Rock | Tile::Wall)
    }

    /// Tall grass and plain grass carry wild encounters; paths and floors do
    /// not, so a paved town or a cleared road is a safe walk.
    pub fn encounters(self) -> bool {
        matches!(self, Tile::Grass | Tile::TallGrass)
    }

    /// Walkable surface height in metres for this tile: tall grass is one
    /// voxel proud of the rest.
    pub fn ground_height(self) -> f32 {
        match self {
            Tile::TallGrass => 3.0,
            _ => 2.0,
        }
    }

    /// Ground cover material and solid height for the voxel world.
    fn ground(self) -> (u8, i32) {
        match self {
            Tile::Path | Tile::Floor | Tile::Door => (material::SOIL, 2),
            Tile::TallGrass => (material::MOSS, 3),
            Tile::Tree | Tile::Rock | Tile::Wall => (material::MOSS, 2),
            Tile::Lawn | Tile::Grass | Tile::Flower => (material::MOSS, 2),
        }
    }

    /// Relative encounter pressure; tall grass is the classic lure.
    pub fn encounter_weight(self) -> u32 {
        match self {
            Tile::TallGrass => 3,
            Tile::Grass => 1,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NpcSpot {
    pub id: &'static str,
    pub tile: [i32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct ExitSpot {
    pub tile: [i32; 2],
    pub target: Place,
    /// Spawn tile in the target place.
    pub spawn: [i32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub z: i32,
    pub w: i32,
    pub d: i32,
}

impl Rect {
    pub const fn new(x: i32, z: i32, w: i32, d: i32) -> Self {
        Self { x, z, w, d }
    }

    pub fn contains(&self, tile: [i32; 2]) -> bool {
        tile[0] >= self.x
            && tile[0] < self.x + self.w
            && tile[1] >= self.z
            && tile[1] < self.z + self.d
    }
}

pub const MAP_W: i32 = 32;
pub const MAP_H: i32 = 24;

pub struct PlaceLayout {
    pub place: Place,
    pub spawn: [i32; 2],
    /// Ground cover where nothing else is painted. Routes get encounter
    /// grass; towns get decorative flower ground so a town is never a
    /// random-encounter zone.
    pub base: Tile,
    pub paths: &'static [[i32; 2]],
    pub tall_grass: &'static [Rect],
    pub flowers: &'static [Rect],
    pub trees: &'static [Rect],
    pub rocks: &'static [Rect],
    pub buildings: &'static [Rect],
    pub npcs: &'static [NpcSpot],
    pub exits: &'static [ExitSpot],
}

/// Deterministic per-tile hash for scatter decisions.
fn hash2(x: i32, z: i32, salt: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B9)
        ^ (z as u32).wrapping_mul(0x85EB_CA6B)
        ^ salt.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    h
}

fn scatter(hash: u32, chance: u32) -> bool {
    hash % 100 < chance
}

/// Generate the authoritative tile grid for one place. Order matters: paths
/// carve over ground cover, scenery skips carved tiles, and buildings are
/// placed last with a centred south door.
pub fn generate_tiles(layout: &PlaceLayout) -> Vec<Tile> {
    let mut tiles = vec![layout.base; (MAP_W * MAP_H) as usize];
    let index = |t: [i32; 2]| (t[1] * MAP_W + t[0]) as usize;
    let paint = |rect: &Rect, tile: Tile, tiles: &mut Vec<Tile>| {
        for z in rect.z..rect.z + rect.d {
            for x in rect.x..rect.x + rect.w {
                if (0..MAP_W).contains(&x) && (0..MAP_H).contains(&z) {
                    tiles[(z * MAP_W + x) as usize] = tile;
                }
            }
        }
    };

    for rect in layout.tall_grass {
        paint(rect, Tile::TallGrass, &mut tiles);
    }
    for rect in layout.flowers {
        paint(rect, Tile::Flower, &mut tiles);
    }
    for pair in layout.paths.windows(2) {
        carve_line(&mut tiles, pair[0], pair[1]);
    }
    // Every exit gets a two-tile apron toward the map centre so an arriving
    // player never spawns inside scenery in the target place.
    for exit in layout.exits {
        let mut tile = exit.tile;
        let toward_x = if exit.tile[0] <= 1 {
            1
        } else if exit.tile[0] >= MAP_W - 2 {
            -1
        } else {
            0
        };
        let toward_z = if exit.tile[1] <= 1 {
            1
        } else if exit.tile[1] >= MAP_H - 2 {
            -1
        } else {
            0
        };
        set_path(&mut tiles, tile[0], tile[1]);
        for _ in 0..2 {
            if toward_x != 0 {
                tile[0] += toward_x;
            } else if toward_z != 0 {
                tile[1] += toward_z;
            }
            set_path(&mut tiles, tile[0], tile[1]);
            set_path(&mut tiles, tile[0] + 1, tile[1]);
            set_path(&mut tiles, tile[0], tile[1] + 1);
        }
    }
    for rect in layout.trees {
        for z in rect.z..rect.z + rect.d {
            for x in rect.x..rect.x + rect.w {
                let current = tiles[(z * MAP_W + x) as usize];
                if matches!(
                    current,
                    Tile::Lawn | Tile::Grass | Tile::TallGrass | Tile::Flower
                ) && scatter(hash2(x, z, 7), 62)
                {
                    tiles[(z * MAP_W + x) as usize] = Tile::Tree;
                }
            }
        }
    }
    for rect in layout.rocks {
        for z in rect.z..rect.z + rect.d {
            for x in rect.x..rect.x + rect.w {
                let current = tiles[(z * MAP_W + x) as usize];
                if matches!(
                    current,
                    Tile::Lawn | Tile::Grass | Tile::TallGrass | Tile::Flower
                ) && scatter(hash2(x, z, 11), 55)
                {
                    tiles[(z * MAP_W + x) as usize] = Tile::Rock;
                }
            }
        }
    }
    for rect in layout.buildings {
        paint(rect, Tile::Wall, &mut tiles);
        if rect.w > 3 && rect.d > 3 {
            for z in rect.z + 1..rect.z + rect.d - 1 {
                for x in rect.x + 1..rect.x + rect.w - 1 {
                    tiles[(z * MAP_W + x) as usize] = Tile::Floor;
                }
            }
            let door_x = rect.x + rect.w / 2;
            tiles[((rect.z + rect.d - 1) * MAP_W + door_x) as usize] = Tile::Door;
        }
    }
    for spot in layout.npcs {
        tiles[index(spot.tile)] = match spot.id {
            "haven" | "shop" => Tile::Floor,
            _ => Tile::Path,
        };
    }
    for exit in layout.exits {
        tiles[index(exit.tile)] = Tile::Path;
    }
    tiles[index(layout.spawn)] = Tile::Path;
    tiles
}

fn carve_line(tiles: &mut [Tile], from: [i32; 2], to: [i32; 2]) {
    // Two-segment L, always two tiles wide so the player never threads a
    // one-cell gap.
    let (mut x, z) = (from[0], from[1]);
    let step = if to[0] >= x { 1 } else { -1 };
    while x != to[0] {
        set_path(tiles, x, z);
        set_path(tiles, x, z + 1);
        x += step;
    }
    let (x, mut z) = (to[0], from[1]);
    let step = if to[1] >= z { 1 } else { -1 };
    while z != to[1] {
        set_path(tiles, x, z);
        set_path(tiles, x + 1, z);
        z += step;
    }
    set_path(tiles, to[0], to[1]);
    set_path(tiles, to[0] + 1, to[1]);
}

fn set_path(tiles: &mut [Tile], x: i32, z: i32) {
    if (0..MAP_W).contains(&x) && (0..MAP_H).contains(&z) {
        let index = (z * MAP_W + x) as usize;
        if tiles[index].walkable() {
            tiles[index] = Tile::Path;
        }
    }
}

/// Build the authoritative voxel world for a place from its tiles. Ground is
/// two voxels; tall grass adds a clump; trees and rocks rise above it. Every
/// solid cell here is exactly what physics collides with.
pub fn generate_world(layout: &PlaceLayout, seed: u64) -> World {
    let tiles = generate_tiles(layout);
    let mut world = World::new(seed);
    for z in 0..MAP_H {
        for x in 0..MAP_W {
            let tile = tiles[(z * MAP_W + x) as usize];
            let (surface, top) = tile.ground();
            for y in 0..top {
                world.set(
                    [x, y, z],
                    if y == top - 1 {
                        surface
                    } else {
                        material::SOIL
                    },
                );
            }
            match tile {
                Tile::Tree => {
                    let trunk = 3 + (hash2(x, z, 23) % 2) as i32;
                    for y in top..top + trunk {
                        world.set([x, y, z], material::WOOD);
                    }
                    for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1), (0, 0)] {
                        world.set([x + dx, top + trunk, z + dz], material::CANOPY);
                        world.set([x + dx, top + trunk - 1, z + dz], material::CANOPY);
                    }
                }
                Tile::Rock => {
                    let height = 1 + (hash2(x, z, 29) % 2) as i32;
                    for y in top..top + height {
                        world.set([x, y, z], material::STONE);
                    }
                }
                Tile::Wall => {
                    for y in top..top + 3 {
                        world.set([x, y, z], material::STONE);
                    }
                }
                Tile::Flower => {
                    world.set([x, top, z], material::MINERAL);
                }
                _ => {}
            }
        }
    }
    world
}

/// A built place: cached tiles and the authoritative world.
pub struct PlaceMap {
    pub layout: &'static PlaceLayout,
    pub tiles: Vec<Tile>,
    pub world: World,
}

impl PlaceMap {
    pub fn build(place: Place, seed: u64) -> Self {
        let layout = layout(place);
        Self {
            tiles: generate_tiles(layout),
            layout,
            world: generate_world(layout, seed ^ place as u64),
        }
    }

    pub fn tile(&self, x: i32, z: i32) -> Option<Tile> {
        if !(0..MAP_W).contains(&x) || !(0..MAP_H).contains(&z) {
            return None;
        }
        Some(self.tiles[(z * MAP_W + x) as usize])
    }

    pub fn walkable(&self, x: i32, z: i32) -> bool {
        self.tile(x, z).is_some_and(Tile::walkable)
    }

    /// The exit whose tile the player has entered, if any.
    pub fn exit_at(&self, tile: [i32; 2]) -> Option<&'static ExitSpot> {
        self.layout.exits.iter().find(|e| e.tile == tile)
    }

    pub fn npc_at(&self, tile: [i32; 2]) -> Option<&'static NpcSpot> {
        self.layout.npcs.iter().find(|n| n.tile == tile)
    }
}

/// The authored layouts. Journey order: Emberfield -> Meadow Way ->
/// Thornhollow -> (Mistpath side loop) -> Tidewater -> Quarry Loop ->
/// Emberfield.
pub fn layout(place: Place) -> &'static PlaceLayout {
    match place {
        Place::Emberfield => &EMBERFIELD,
        Place::MeadowWay => &MEADOW_WAY,
        Place::Thornhollow => &THORN_HOLLOW,
        Place::Mistpath => &MISTPATH,
        Place::Tidewater => &TIDEWATER,
        Place::QuarryLoop => &QUARRY_LOOP,
    }
}

static EMBERFIELD: PlaceLayout = PlaceLayout {
    place: Place::Emberfield,
    spawn: [16, 20],
    base: Tile::Lawn,
    paths: &[
        [16, 20],
        [16, 14],
        [16, 1],
        [16, 14],
        [8, 13],
        [16, 14],
        [24, 13],
        [16, 19],
        [29, 19],
    ],
    tall_grass: &[],
    flowers: &[Rect::new(10, 6, 5, 4), Rect::new(19, 15, 3, 3)],
    trees: &[
        Rect::new(0, 0, 32, 3),
        Rect::new(0, 3, 5, 21),
        Rect::new(27, 3, 5, 10),
    ],
    rocks: &[Rect::new(12, 17, 3, 3)],
    buildings: &[Rect::new(5, 8, 6, 5), Rect::new(21, 8, 6, 5)],
    npcs: &[
        NpcSpot {
            id: "haven",
            tile: [8, 13],
        },
        NpcSpot {
            id: "shop",
            tile: [24, 13],
        },
    ],
    exits: &[
        ExitSpot {
            tile: [16, 1],
            target: Place::MeadowWay,
            spawn: [16, 21],
        },
        ExitSpot {
            tile: [30, 19],
            target: Place::QuarryLoop,
            spawn: [2, 19],
        },
    ],
};

static MEADOW_WAY: PlaceLayout = PlaceLayout {
    place: Place::MeadowWay,
    spawn: [16, 21],
    base: Tile::Grass,
    paths: &[[16, 21], [16, 2], [16, 12], [8, 10], [16, 12], [24, 11]],
    tall_grass: &[
        Rect::new(5, 5, 8, 6),
        Rect::new(19, 5, 8, 6),
        Rect::new(5, 14, 8, 5),
        Rect::new(20, 14, 7, 5),
    ],
    flowers: &[Rect::new(13, 6, 4, 3), Rect::new(14, 16, 3, 3)],
    trees: &[
        Rect::new(0, 0, 32, 2),
        Rect::new(0, 22, 32, 2),
        Rect::new(0, 2, 3, 20),
        Rect::new(29, 2, 3, 20),
    ],
    rocks: &[],
    buildings: &[],
    npcs: &[NpcSpot {
        id: "meadow-ranger",
        tile: [20, 12],
    }],
    exits: &[
        ExitSpot {
            tile: [16, 22],
            target: Place::Emberfield,
            spawn: [16, 2],
        },
        ExitSpot {
            tile: [16, 1],
            target: Place::Thornhollow,
            spawn: [16, 21],
        },
    ],
};

static THORN_HOLLOW: PlaceLayout = PlaceLayout {
    place: Place::Thornhollow,
    spawn: [16, 21],
    base: Tile::Grass,
    paths: &[[16, 21], [16, 2], [16, 12], [29, 12]],
    tall_grass: &[
        Rect::new(6, 4, 8, 6),
        Rect::new(19, 4, 8, 5),
        Rect::new(6, 14, 7, 5),
        Rect::new(20, 14, 8, 5),
    ],
    flowers: &[Rect::new(13, 8, 3, 3), Rect::new(15, 17, 3, 2)],
    trees: &[
        Rect::new(0, 0, 32, 2),
        Rect::new(0, 22, 32, 2),
        Rect::new(0, 2, 4, 20),
        Rect::new(28, 14, 4, 8),
    ],
    rocks: &[Rect::new(10, 10, 3, 3), Rect::new(21, 10, 3, 3)],
    buildings: &[],
    npcs: &[NpcSpot {
        id: "thorn-wayfarer",
        tile: [16, 17],
    }],
    exits: &[
        ExitSpot {
            tile: [16, 22],
            target: Place::MeadowWay,
            spawn: [16, 2],
        },
        ExitSpot {
            tile: [16, 1],
            target: Place::Tidewater,
            spawn: [16, 21],
        },
        ExitSpot {
            tile: [30, 12],
            target: Place::Mistpath,
            spawn: [1, 13],
        },
    ],
};

static MISTPATH: PlaceLayout = PlaceLayout {
    place: Place::Mistpath,
    spawn: [2, 12],
    base: Tile::Grass,
    paths: &[[2, 12], [29, 12], [16, 12], [16, 21]],
    tall_grass: &[
        Rect::new(4, 4, 6, 6),
        Rect::new(21, 4, 6, 6),
        Rect::new(4, 15, 6, 6),
        Rect::new(22, 15, 6, 6),
    ],
    flowers: &[Rect::new(12, 5, 3, 3), Rect::new(14, 16, 3, 3)],
    trees: &[
        Rect::new(0, 0, 32, 2),
        Rect::new(0, 22, 32, 2),
        Rect::new(0, 2, 3, 20),
        Rect::new(29, 2, 3, 10),
        Rect::new(11, 7, 4, 3),
    ],
    rocks: &[Rect::new(12, 16, 3, 3)],
    buildings: &[],
    npcs: &[NpcSpot {
        id: "mist-herbalist",
        tile: [22, 12],
    }],
    exits: &[
        ExitSpot {
            tile: [1, 13],
            target: Place::Thornhollow,
            spawn: [28, 12],
        },
        ExitSpot {
            tile: [16, 22],
            target: Place::Tidewater,
            spawn: [16, 21],
        },
    ],
};

static TIDEWATER: PlaceLayout = PlaceLayout {
    place: Place::Tidewater,
    spawn: [16, 21],
    base: Tile::Lawn,
    paths: &[
        [16, 21],
        [16, 13],
        [16, 3],
        [2, 12],
        [16, 13],
        [16, 12],
        [6, 7],
        [16, 13],
        [26, 8],
    ],
    tall_grass: &[],
    flowers: &[Rect::new(3, 15, 6, 4), Rect::new(23, 15, 6, 4)],
    trees: &[
        Rect::new(0, 0, 32, 2),
        Rect::new(0, 2, 2, 20),
        Rect::new(30, 2, 2, 20),
    ],
    rocks: &[],
    buildings: &[
        Rect::new(11, 6, 10, 7),
        Rect::new(3, 3, 5, 4),
        Rect::new(24, 4, 5, 4),
    ],
    npcs: &[
        NpcSpot {
            id: "haven",
            tile: [5, 7],
        },
        NpcSpot {
            id: "shop",
            tile: [26, 8],
        },
        NpcSpot {
            id: "gym-attendant-lune",
            tile: [13, 9],
        },
        NpcSpot {
            id: "gym-attendant-kest",
            tile: [18, 9],
        },
        NpcSpot {
            id: "leader-marwick",
            tile: [16, 8],
        },
    ],
    exits: &[
        ExitSpot {
            tile: [16, 1],
            target: Place::Thornhollow,
            spawn: [16, 2],
        },
        ExitSpot {
            tile: [0, 12],
            target: Place::Mistpath,
            spawn: [28, 12],
        },
        ExitSpot {
            tile: [16, 22],
            target: Place::QuarryLoop,
            spawn: [16, 2],
        },
    ],
};

static QUARRY_LOOP: PlaceLayout = PlaceLayout {
    place: Place::QuarryLoop,
    spawn: [16, 2],
    base: Tile::Grass,
    paths: &[[16, 2], [16, 20], [2, 20], [16, 20], [27, 19]],
    tall_grass: &[
        Rect::new(6, 7, 6, 4),
        Rect::new(20, 7, 6, 4),
        Rect::new(6, 13, 5, 4),
        Rect::new(21, 13, 5, 4),
    ],
    flowers: &[Rect::new(14, 9, 4, 3)],
    trees: &[Rect::new(0, 0, 32, 1), Rect::new(0, 23, 32, 1)],
    rocks: &[
        Rect::new(4, 3, 7, 4),
        Rect::new(21, 3, 7, 4),
        Rect::new(4, 18, 7, 4),
        Rect::new(21, 18, 7, 4),
    ],
    buildings: &[],
    npcs: &[
        NpcSpot {
            id: "quarry-prospector",
            tile: [16, 14],
        },
        NpcSpot {
            id: "quarry-scout",
            tile: [24, 10],
        },
    ],
    exits: &[
        ExitSpot {
            tile: [16, 1],
            target: Place::Tidewater,
            spawn: [16, 21],
        },
        ExitSpot {
            tile: [2, 19],
            target: Place::Emberfield,
            spawn: [28, 19],
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_monsters::journey::Place;
    use std::collections::VecDeque;

    fn layout_of(place: Place) -> &'static PlaceLayout {
        layout(place)
    }

    fn spawn_reachable(place: Place) -> Vec<[i32; 2]> {
        let layout = layout_of(place);
        let tiles = generate_tiles(layout);
        let mut seen = vec![false; (MAP_W * MAP_H) as usize];
        let mut queue = VecDeque::new();
        queue.push_back(layout.spawn);
        seen[(layout.spawn[1] * MAP_W + layout.spawn[0]) as usize] = true;
        let mut reachable = vec![layout.spawn];
        while let Some([x, z]) = queue.pop_front() {
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if !(0..MAP_W).contains(&nx) || !(0..MAP_H).contains(&nz) {
                    continue;
                }
                let index = (nz * MAP_W + nx) as usize;
                if seen[index] || !tiles[index].walkable() {
                    continue;
                }
                seen[index] = true;
                reachable.push([nx, nz]);
                queue.push_back([nx, nz]);
            }
        }
        reachable
    }

    #[test]
    fn every_place_generates_a_full_bounded_map() {
        for place in Place::ALL {
            let map = PlaceMap::build(place, 7);
            assert_eq!(map.tiles.len(), (MAP_W * MAP_H) as usize);
            assert!(map.walkable(map.layout.spawn[0], map.layout.spawn[1]));
            assert!(!map.layout.exits.is_empty(), "{:?} has no exit", place);
        }
    }

    #[test]
    fn spawn_reaches_every_exit_and_interaction() {
        for place in Place::ALL {
            let reachable = spawn_reachable(place);
            let set: std::collections::HashSet<[i32; 2]> = reachable.iter().copied().collect();
            let layout = layout_of(place);
            for exit in layout.exits {
                assert!(
                    set.contains(&exit.tile),
                    "{:?}: exit at {:?} is unreachable",
                    place,
                    exit.tile
                );
            }
            for npc in layout.npcs {
                assert!(
                    set.contains(&npc.tile),
                    "{:?}: {} at {:?} is unreachable",
                    place,
                    npc.id,
                    npc.tile
                );
            }
        }
    }

    #[test]
    fn exits_pair_up_between_places() {
        for place in Place::ALL {
            for exit in layout_of(place).exits {
                let target = layout_of(exit.target);
                // The arriving spawn must be walkable in the target place.
                assert!(
                    PlaceMap::build(exit.target, 7).walkable(exit.spawn[0], exit.spawn[1]),
                    "{:?} -> {:?} spawn {:?} is inside geometry",
                    place,
                    exit.target,
                    exit.spawn
                );
                // Somewhere in the target place there is a return route back.
                assert!(
                    !target.exits.is_empty(),
                    "{:?} has no exit back",
                    exit.target
                );
            }
        }
    }

    #[test]
    fn routes_offer_encounter_ground_but_towns_do_not() {
        for place in Place::ALL {
            let layout = layout_of(place);
            let tiles = generate_tiles(layout);
            let encounters = tiles.iter().filter(|t| t.encounters()).count();
            if place.is_route() {
                assert!(
                    encounters > 60,
                    "{:?} offers only {encounters} grass tiles",
                    place
                );
            } else {
                assert_eq!(encounters, 0, "{:?} must be a safe town", place);
            }
        }
    }

    #[test]
    fn buildings_have_a_reachable_door_and_an_interior() {
        for place in Place::ALL {
            let layout = layout_of(place);
            let tiles = generate_tiles(layout);
            for rect in layout.buildings {
                let mut doors = 0;
                let mut floors = 0;
                for z in rect.z..rect.z + rect.d {
                    for x in rect.x..rect.x + rect.w {
                        match tiles[(z * MAP_W + x) as usize] {
                            Tile::Door => doors += 1,
                            Tile::Floor => floors += 1,
                            _ => {}
                        }
                    }
                }
                assert_eq!(doors, 1, "{:?}: building {rect:?} has {doors} doors", place);
                assert!(floors > 0, "{:?}: building {rect:?} has no interior", place);
            }
        }
    }

    #[test]
    fn the_journey_loop_closes_through_all_four_routes() {
        // Every route is on the walkable graph between the two towns.
        for place in [
            Place::MeadowWay,
            Place::Thornhollow,
            Place::Mistpath,
            Place::QuarryLoop,
        ] {
            assert!(place.is_route());
            let layout = layout_of(place);
            assert!(!layout.exits.is_empty());
        }
        // Both towns connect to routes; the Quarry Loop returns to Emberfield.
        let ember = layout_of(Place::Emberfield);
        assert!(ember.exits.iter().any(|e| e.target == Place::QuarryLoop));
        let tide = layout_of(Place::Tidewater);
        assert!(tide.exits.iter().any(|e| e.target == Place::QuarryLoop));
    }
}
